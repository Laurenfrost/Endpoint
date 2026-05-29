//! Tauri 命令:**薄**桥接层。
//!
//! 此层只做参数转换、把核心库调用放到 blocking 线程池、把错误转成前端可消费的字符串。
//! 所有业务逻辑都在 `endpoint-core` 里。
//!
//! 文件选择对话框走 Rust 端的 `tauri-plugin-dialog`,避免前端依赖 plugin 的 JS(纯静态
//! HTML 没有 bundler,`withGlobalTauri` 只暴露 core API,plugin JS 默认不可用)。
//!
//! # 阶段二命令分布
//!
//! - 加载与分析:[`load_and_analyze`] —— 跑 [`endpoint_core::run_pipeline`],返回 JSON DTO
//!   并把完整 [`PipelineOutput`] 缓存到 [`AppState`]。
//! - 构建 EPUB:[`build_epub`] —— 从缓存取 pipeline,叠加用户编辑过的元数据,写文件。
//! - 取消:[`cancel_task`] —— v1 只置位 `AtomicBool`,核心库长循环未实装检查。
//! - 旧 [`convert`] —— 阶段零的一站式入口,保留作回归保险,前端不再调用。
//! - 文件对话框:[`pick_input_file`] / [`pick_output_file`] / [`pick_executable_file`] —— 沿用。
//!
//! # 进度事件契约(详见 `docs/stage2-design.md` 第三节)
//!
//! 事件名:`endpoint://progress`
//! payload:[`ProgressEvent`] —— `{ task_id, stage, percent, detail? }`
//! `stage` 枚举:`"decoding"` / `"cleaning"` / `"chapter"` / `"epub"` / `"kepubify"`
//!
//! v1 颗粒度:每 stage 开始 0%、结束 100%;`decoding` 结束时 detail 携带实际编码。

use std::path::{Path, PathBuf};

use endpoint_core::{
    build_epub_from, convert as core_convert, run_pipeline, ConvertOptions, Metadata,
    PipelineOutput, ProgressSink,
};
use endpoint_core::domain::{
    CleaningAnnotation, CleaningKind, WatermarkSignal, WatermarkSignalKind, WatermarkVerdict,
};
use endpoint_core::llm::AdjudicationVerdict;
use serde::Serialize;
use tauri::{async_runtime, AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tracing::{debug, info, warn};

use crate::state::{AppState, CachedPipeline};

// ============== 进度事件 ==============

#[derive(Serialize, Clone)]
pub struct ProgressEvent {
    pub task_id: String,
    pub stage: String,
    pub percent: u8,
    pub detail: Option<String>,
}

/// 把核心库的 [`ProgressSink::report`] 调用转成 Tauri 事件 `endpoint://progress`。
struct TauriProgressSink {
    app: AppHandle,
    task_id: String,
}

impl ProgressSink for TauriProgressSink {
    fn report(&self, stage: &str, percent: u8, detail: Option<&str>) {
        let _ = self.app.emit(
            "endpoint://progress",
            ProgressEvent {
                task_id: self.task_id.clone(),
                stage: stage.to_string(),
                percent,
                detail: detail.map(|s| s.to_string()),
            },
        );
    }
}

// ============== 文件对话框 ==============

fn file_path_to_string(fp: FilePath) -> Option<String> {
    match fp {
        FilePath::Path(p) => Some(p.display().to_string()),
        FilePath::Url(u) => Some(u.to_string()),
    }
}

#[tauri::command]
pub async fn pick_input_file(app: AppHandle) -> Option<String> {
    async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Text", &["txt"])
            .blocking_pick_file()
            .and_then(file_path_to_string)
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn pick_output_file(
    app: AppHandle,
    default_path: Option<String>,
) -> Option<String> {
    async_runtime::spawn_blocking(move || {
        let mut b = app.dialog().file().add_filter("EPUB", &["epub"]);
        if let Some(p) = default_path {
            b = b.set_file_name(p);
        }
        b.blocking_save_file().and_then(file_path_to_string)
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn pick_executable_file(app: AppHandle) -> Option<String> {
    async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Executable", &["exe"])
            .blocking_pick_file()
            .and_then(file_path_to_string)
    })
    .await
    .ok()
    .flatten()
}

/// 弹出字体文件选择对话框,返回用户选中的路径(ttf / otf)。用户取消返回 null。
#[tauri::command]
pub async fn pick_font_file(app: AppHandle) -> Option<String> {
    async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Font", &["ttf", "otf"])
            .blocking_pick_file()
            .and_then(file_path_to_string)
    })
    .await
    .ok()
    .flatten()
}

/// 弹出封面图片选择对话框,读取文件并以 base64 data URL 形式返回预览 + 路径。
///
/// 返回 `{ path, data_url }` JSON 对象,供前端 `<img src=data_url>` 预览。
/// 用户取消时返回 `null`。
#[tauri::command]
pub async fn pick_cover_file(app: AppHandle) -> Result<Option<serde_json::Value>, String> {
    let path_opt = async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Image", &["jpg", "jpeg", "png"])
            .blocking_pick_file()
            .and_then(file_path_to_string)
    })
    .await
    .map_err(|e| format!("任务调度失败: {e}"))?;

    let Some(path) = path_opt else {
        return Ok(None);
    };

    let bytes = std::fs::read(&path).map_err(|e| format!("读取封面图片失败: {e}"))?;
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let mime = if ext == "png" { "image/png" } else { "image/jpeg" };
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    let data_url = format!("data:{mime};base64,{b64}");

    Ok(Some(serde_json::json!({ "path": path, "dataUrl": data_url })))
}

// ============== 阶段二主入口 ==============

/// 加载 txt 并跑完整管线,返回富标注 JSON DTO,并把完整 [`PipelineOutput`] 缓存到
/// [`AppState`] 供后续 `build_epub` 复用。
///
/// 同一份 `input_path` 重复调用会覆盖上次缓存。
/// `encoding_override` 传 `None` 走自动探测;传 `Some("GBK")` 等强制使用。
///
/// 元数据用占位的空 title/author 跑 pipeline——书名/作者从前端阶段 4 表单送回,届时在
/// [`build_epub`] 中覆盖 metadata。这样避免阶段 1 强制用户先填元数据。
#[tauri::command]
pub async fn load_and_analyze(
    app: AppHandle,
    state: State<'_, AppState>,
    input_path: String,
    encoding_override: Option<String>,
    cleaning_config: Option<serde_json::Value>,
    watermark_config: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let task_id = state.next_task_id("load");
    info!(
        task_id = %task_id,
        input = %input_path,
        encoding_override = encoding_override.as_deref().unwrap_or("auto"),
        "load_and_analyze 开始"
    );
    let cancel_flag = state.register_cancel(&task_id);

    // 阶段三 v2:把前端传的 cleaning_config / watermark_config(JSON 对象)
    // 反序列化为对应 Config。反序列化失败时返回错误,不静默 fallback——避免
    // 用户改了配置但实际未生效。
    let cleaning_cfg = match cleaning_config {
        Some(v) => Some(
            serde_json::from_value::<endpoint_core::cleaning::CleaningConfig>(v)
                .map_err(|e| format!("cleaning_config 反序列化失败: {e}"))?,
        ),
        None => None,
    };
    let watermark_cfg = match watermark_config {
        Some(v) => Some(
            serde_json::from_value::<endpoint_core::watermark::WatermarkConfig>(v)
                .map_err(|e| format!("watermark_config 反序列化失败: {e}"))?,
        ),
        None => None,
    };

    let app_for_sink = app.clone();
    let path_for_blocking = input_path.clone();
    let task_id_for_blocking = task_id.clone();

    let pipeline_result = async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path_for_blocking).map_err(|e| {
            warn!(error = %e, task_id = %task_id_for_blocking, "读取文件失败");
            format!("读取文件失败({path_for_blocking}): {e}")
        })?;
        debug!(bytes = bytes.len(), task_id = %task_id_for_blocking, "文件读取完成,进入核心管线");
        // 4.8:若用户已保存过 LLM 归纳规则,自动合并进分析(save_induced_rule 写到同目录)
        let user_rules = crate::llm_config::user_rules_path().filter(|p| p.exists());
        let options = ConvertOptions {
            encoding_override,
            rules_path: user_rules,
            kepubify_path: None,
            cancel_token: Some(cancel_flag),
            watermark: watermark_cfg,
            cleaning: cleaning_cfg,
            css_override: None,
            embed_fonts: false,
            font_bytes: None,
        };
        // 阶段 1 不强制元数据,先用占位;阶段 4 通过 build_epub 的 title/author 参数覆盖。
        let metadata = Metadata::new("", "");
        let sink = TauriProgressSink {
            app: app_for_sink,
            task_id: task_id_for_blocking,
        };
        run_pipeline(&bytes, metadata, &options, &sink).map_err(|e| format!("{e}"))
    })
    .await;

    state.unregister_cancel(&task_id);

    let pipeline: PipelineOutput = pipeline_result
        .map_err(|e| {
            warn!(error = %e, task_id = %task_id, "任务调度失败");
            format!("任务调度失败: {e}")
        })?
        .map_err(|e| {
            warn!(error = %e, task_id = %task_id, "管线解析失败");
            format!("解析失败: {e}")
        })?;

    let dto = serde_json::to_value(&pipeline)
        .map_err(|e| format!("序列化 PipelineOutput 失败: {e}"))?;
    info!(
        task_id = %task_id,
        encoding = %pipeline.source_encoding,
        cleaning = pipeline.cleaning.len(),
        watermark = pipeline.watermark.len(),
        entries = pipeline.book.entries.len(),
        "load_and_analyze 完成"
    );

    *state
        .pipeline
        .lock()
        .map_err(|e| format!("pipeline 锁中毒: {e}"))? = Some(CachedPipeline {
        source_path: input_path,
        output: pipeline,
    });

    Ok(dto)
}

/// 用最近一次缓存的 [`PipelineOutput`] 构建 EPUB,可选 kepubify。
///
/// `title` / `author` 覆盖缓存中元数据(`load_and_analyze` 时是占位)。
/// 调用前必须先 `load_and_analyze`,否则返回错误。
///
/// **阶段三 v2.2 新增** `decisions`:用户对自动检测结果的覆盖决策列表(可选)。
/// 若提供,会在构建 EPUB 前调用 [`watermark::apply_user_decisions`] 重组 cleaning
/// + 重 materialize paragraphs,从而让"用户拒绝的清洗"留在 EPUB、"用户接受的 suspect"
/// 从 EPUB 删掉。决策仅本次调用有效,不持久化。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn build_epub(
    app: AppHandle,
    state: State<'_, AppState>,
    output_path: String,
    title: String,
    author: String,
    kepubify_path: Option<String>,
    decisions: Option<Vec<serde_json::Value>>,
    cover_path: Option<String>,
    css_override: Option<String>,
    embed_fonts: Option<bool>,
    font_path: Option<String>,
    // 阶段四扩展元数据(全部可选,未填则 EPUB 不写对应项)
    description: Option<String>,
    subjects: Option<Vec<String>>,
    series: Option<String>,
    series_index: Option<u32>,
) -> Result<String, String> {
    // 反序列化决策列表(失败显式报错,不静默)
    let decisions_typed: Vec<endpoint_core::domain::UserDecision> = match decisions {
        Some(list) => list
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                serde_json::from_value(v)
                    .map_err(|e| format!("decisions[{i}] 反序列化失败: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    // 把 pipeline 拷贝出来(改 metadata 不影响缓存原件),交给 blocking 闭包。
    let pipeline_for_build: PipelineOutput = {
        let guard = state
            .pipeline
            .lock()
            .map_err(|e| format!("pipeline 锁中毒: {e}"))?;
        let cached = guard
            .as_ref()
            .ok_or_else(|| "尚未加载文件,请先调用 load_and_analyze".to_string())?;
        let mut out = cached.output.clone();
        out.book.metadata.title = title;
        out.book.metadata.author = author;
        // 扩展元数据:仅在前端传了非空值时写入,空字符串 / 空数组视为「未填」。
        if let Some(d) = description.as_ref().filter(|s| !s.trim().is_empty()) {
            out.book.metadata.description = Some(d.trim().to_string());
        }
        if let Some(subs) = subjects {
            let cleaned: Vec<String> = subs
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !cleaned.is_empty() {
                out.book.metadata.subjects = cleaned;
            }
        }
        if let Some(s) = series.as_ref().filter(|s| !s.trim().is_empty()) {
            out.book.metadata.series = Some(s.trim().to_string());
            out.book.metadata.series_index = series_index;
        }
        out
    };

    // 在主线程读封面字节(IO 在 blocking 里也行,但封面文件通常很小)
    let cover_bytes: Option<Vec<u8>> = match &cover_path {
        Some(p) => Some(std::fs::read(p).map_err(|e| format!("读取封面图片失败: {e}"))?),
        None => None,
    };
    let cover_mime_str = cover_path.as_deref().and_then(|p| {
        std::path::Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
    });

    // 字体字节读取(embed_fonts=true 时)
    let font_bytes_owned: Option<endpoint_core::epub::FontBytes> = if embed_fonts.unwrap_or(false) {
        if let Some(custom_path) = &font_path {
            let bytes = std::fs::read(custom_path)
                .map_err(|e| format!("读取自定义字体失败: {e}"))?;
            let name = std::path::Path::new(custom_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("CustomFont")
                .to_string();
            Some(endpoint_core::epub::FontBytes { name, regular: bytes })
        } else {
            let resource_path = app
                .path()
                .resource_dir()
                .map_err(|e| format!("无法获取资源目录: {e}"))?
                .join("fonts/LXGWWenKai-Regular.ttf");
            let bytes = std::fs::read(&resource_path).map_err(|e| {
                format!("读取内置字体失败(请先运行 scripts/fetch-fonts.ps1): {e}")
            })?;
            Some(endpoint_core::epub::FontBytes {
                name: "LXGWWenKai".to_string(),
                regular: bytes,
            })
        }
    } else {
        None
    };

    let task_id = state.next_task_id("build");
    info!(
        task_id = %task_id,
        output = %output_path,
        kepubify = kepubify_path.is_some(),
        decisions = decisions_typed.len(),
        embed_fonts = embed_fonts.unwrap_or(false),
        has_cover = cover_bytes.is_some(),
        "build_epub 开始"
    );
    let _cancel_flag = state.register_cancel(&task_id);

    let app_for_sink = app.clone();
    let task_id_for_blocking = task_id.clone();
    let kepubify_owned = kepubify_path.map(PathBuf::from);
    let output_owned = PathBuf::from(output_path);

    let build_result = async_runtime::spawn_blocking(move || {
        let sink = TauriProgressSink {
            app: app_for_sink,
            task_id: task_id_for_blocking,
        };

        // v2.2:若有决策,叠加到 cleaning + 重新 materialize paragraphs。
        // 没决策走老路径(等价于"全部保持默认"),性能与 v2.1 相同。
        let mut pipeline = pipeline_for_build;
        if !decisions_typed.is_empty() {
            let new_cleaning = endpoint_core::watermark::apply_user_decisions(
                &pipeline.cleaning,
                &pipeline.watermark,
                &decisions_typed,
            );
            pipeline.cleaning = new_cleaning;
            endpoint_core::chapter::materialize_paragraphs(
                &mut pipeline.book,
                &pipeline.source_text,
                &pipeline.cleaning,
            );
        }

        // 构造 EpubOptions:封面 + CSS 覆盖 + 字体嵌入
        let cover_mime = cover_mime_str
            .as_deref()
            .and_then(endpoint_core::epub::CoverMime::from_path_ext)
            .unwrap_or(endpoint_core::epub::CoverMime::Jpeg);
        let epub_opts = endpoint_core::epub::EpubOptions {
            css_override: css_override.as_deref(),
            cover: cover_bytes.as_deref(),
            cover_mime,
            font_bytes: font_bytes_owned.as_ref(),
        };

        build_epub_from(
            &pipeline,
            &output_owned,
            kepubify_owned.as_deref(),
            &epub_opts,
            &sink,
        )
        .map_err(|e| format!("{e}"))
    })
    .await;

    state.unregister_cancel(&task_id);

    let final_path = build_result
        .map_err(|e| {
            warn!(error = %e, task_id = %task_id, "build 任务调度失败");
            format!("任务调度失败: {e}")
        })?
        .map_err(|e| {
            warn!(error = %e, task_id = %task_id, "EPUB 构建失败");
            format!("构建失败: {e}")
        })?;

    info!(task_id = %task_id, path = %final_path.display(), "build_epub 完成");
    Ok(final_path.display().to_string())
}

/// 取消任务:v1 只置位标志,核心库长循环未实装检查(参见 `chapter.rs` / `cleaning.rs`
/// 的 `TODO(cancel)` 注释)。前端调用后任务实际仍会跑完——但接口已就位,阶段三/四
/// 可在不改命令签名的情况下补足。
#[tauri::command]
pub async fn cancel_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let guard = state
        .cancel_flags
        .lock()
        .map_err(|e| format!("cancel_flags 锁中毒: {e}"))?;
    if let Some(flag) = guard.get(&task_id) {
        flag.store(true, Ordering::Relaxed);
        warn!(task_id = %task_id, "用户取消任务");
    } else {
        debug!(task_id = %task_id, "取消时未找到对应任务标志,可能已完成");
    }
    Ok(())
}

// ============== 阶段四 4.2:CSS 主题 ==============

/// 列出可用主题名称列表(resource themes/ 目录下所有 .css 文件的 stem)。
/// 顺序固定:easypub → standard → classic → highcontrast;其余按字母序追加。
#[tauri::command]
pub async fn list_themes(app: AppHandle) -> Result<Vec<String>, String> {
    let themes_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("无法获取资源目录: {e}"))?
        .join("themes");

    let mut names: Vec<String> = std::fs::read_dir(&themes_dir)
        .map_err(|e| format!("读取主题目录失败: {e}"))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? == "css" {
                Some(path.file_stem()?.to_str()?.to_string())
            } else {
                None
            }
        })
        .collect();

    // 把内置主题置前(默认 easypub 在最前),其余按字母序
    let priority = ["easypub", "standard", "classic", "highcontrast"];
    names.sort_by(|a, b| {
        let ia = priority.iter().position(|&p| p == a).unwrap_or(usize::MAX);
        let ib = priority.iter().position(|&p| p == b).unwrap_or(usize::MAX);
        ia.cmp(&ib).then(a.cmp(b))
    });
    Ok(names)
}

/// 读取指定主题的 CSS 文本内容。`name` 不含 `.css` 扩展名。
#[tauri::command]
pub async fn load_theme(app: AppHandle, name: String) -> Result<String, String> {
    // 防止路径穿越
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("主题名称不合法: {name}"));
    }
    let path = app
        .path()
        .resource_dir()
        .map_err(|e| format!("无法获取资源目录: {e}"))?
        .join("themes")
        .join(format!("{name}.css"));

    std::fs::read_to_string(&path)
        .map_err(|e| format!("读取主题 {name} 失败: {e}"))
}

// ============== 阶段四 4.3:文字封面自动生成 ==============

/// 用字体渲染「书名 + 作者」生成 1400×2100 PNG 封面，返回 `{ path, dataUrl }`。
///
/// `font_path` 为 `None` 时使用内置霞鹜文楷（需先运行 fetch-fonts.ps1）。
/// `style` 取 `"default"`（深蓝）或 `"gradient"`（蓝紫）。
#[tauri::command]
pub async fn generate_text_cover(
    app: AppHandle,
    title: String,
    author: String,
    style: String,
    font_path: Option<String>,
) -> Result<serde_json::Value, String> {
    let font_bytes = if let Some(custom) = font_path {
        std::fs::read(&custom).map_err(|e| format!("读取字体失败: {e}"))?
    } else {
        let resource_path = app
            .path()
            .resource_dir()
            .map_err(|e| format!("无法获取资源目录: {e}"))?
            .join("fonts/LXGWWenKai-Regular.ttf");
        std::fs::read(&resource_path).map_err(|e| {
            format!("读取内置字体失败(请先运行 scripts/fetch-fonts.ps1): {e}")
        })?
    };

    let cover_style = match style.as_str() {
        "gradient" => endpoint_core::cover_gen::TextCoverStyle::Gradient,
        _ => endpoint_core::cover_gen::TextCoverStyle::Default,
    };

    let png_bytes = async_runtime::spawn_blocking(move || {
        let opts = endpoint_core::cover_gen::TextCoverOptions {
            title: &title,
            author: &author,
            font_bytes: &font_bytes,
            style: cover_style,
        };
        endpoint_core::cover_gen::generate(&opts).map_err(|e| format!("封面生成失败: {e}"))
    })
    .await
    .map_err(|e| format!("任务调度失败: {e}"))??;

    let temp_path = std::env::temp_dir().join("endpoint_text_cover.png");
    std::fs::write(&temp_path, &png_bytes)
        .map_err(|e| format!("写入临时文件失败: {e}"))?;

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
    let data_url = format!("data:image/png;base64,{b64}");

    Ok(serde_json::json!({
        "path": temp_path.display().to_string(),
        "dataUrl": data_url,
    }))
}

// ============== 阶段四 4.5:LLM 配置 ==============

/// 读取当前 LLM + 搜索配置。
///
/// 字段:
/// - `base_url` / `model`:LLM 配置
/// - `key_set` / `key_masked`:LLM API key 状态 / 脱敏显示
/// - `configured`:**LLM 能否真正使用**(`base_url` 与 `api_key` 都非空)
/// - `search_provider`:搜索后端标识(目前仅 `"brave"`)
/// - `search_key_set` / `search_key_masked`:搜索 API key 状态
/// - `search_configured`:**搜索能否真正使用**(provider 与 key 都非空)
///
/// 前端的门控决策应使用 `configured` / `search_configured`,不要用 key_set。
#[tauri::command]
pub async fn get_llm_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = crate::llm_config::load();
    let search_cfg = crate::llm_config::load_search();
    let key_set = !cfg.api_key.is_empty();
    let configured = key_set && !cfg.base_url.is_empty();
    let key_masked = mask(&cfg.api_key);
    let search_key_set = !search_cfg.api_key.is_empty();
    let search_configured = search_key_set && !search_cfg.provider.is_empty();
    let search_key_masked = mask(&search_cfg.api_key);
    let _guard = state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))?;
    drop(_guard);
    Ok(serde_json::json!({
        "base_url": cfg.base_url,
        "model": cfg.model,
        "key_set": key_set,
        "key_masked": key_masked,
        "configured": configured,
        "search_provider": search_cfg.provider,
        "search_key_set": search_key_set,
        "search_key_masked": search_key_masked,
        "search_configured": search_configured,
    }))
}

fn mask(k: &str) -> String {
    if k.is_empty() {
        return String::new();
    }
    if k.len() > 8 {
        format!("{}...{}", &k[..4], &k[k.len() - 4..])
    } else {
        "***".to_string()
    }
}

/// 保存 LLM 配置到 `config.toml`,并立即以新配置重建客户端。
///
/// - `api_key` 传**空字符串**表示「保留磁盘上已有 key」,与 UI placeholder 承诺一致。
/// - 要真正清除 key,前端可显式发送一个非空字符串(目前 UI 没有这个入口,等需要再加)。
#[tauri::command]
pub async fn set_llm_config(
    state: State<'_, AppState>,
    base_url: String,
    model: String,
    api_key: String,
) -> Result<(), String> {
    let trimmed_key = api_key.trim();
    let key_from_user_input = !trimmed_key.is_empty();
    let effective_key = if key_from_user_input {
        trimmed_key.to_string()
    } else {
        crate::llm_config::load().api_key
    };
    let cfg = crate::llm_config::LlmConfig {
        base_url: base_url.trim().to_string(),
        model: model.trim().to_string(),
        api_key: effective_key,
    };
    info!(
        base_url = %cfg.base_url,
        model = %cfg.model,
        key_set = !cfg.api_key.is_empty(),
        key_from_user_input,
        "保存 LLM 配置"
    );
    crate::llm_config::save(&cfg)?;
    // 重建客户端时把当前搜索配置一起注入(允许「只改 LLM 不动搜索」的场景)。
    // 必须 spawn_blocking:reqwest::blocking::Client::builder().build() 内部短暂
    // 起一个 tokio 运行时,在外层 async 上下文里析构会 panic。同 suggest_metadata 的坑。
    let new_client = async_runtime::spawn_blocking(move || {
        let search_cfg = crate::llm_config::load_search();
        crate::llm_config::create_client(&cfg, &search_cfg)
    })
    .await
    .map_err(|e| format!("构造 LLM 客户端任务失败: {e}"))?;
    *state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))? = new_client;
    Ok(())
}

/// 保存搜索后端配置并热替换 LLM 客户端中的搜索依赖。
///
/// `provider` 传空字符串 = 禁用搜索(LLM 仍可用,只是 Pass B 永不触发)。
/// `api_key` 传空字符串遵循同 set_llm_config 的「保留旧值」语义。
#[tauri::command]
pub async fn set_search_config(
    state: State<'_, AppState>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    let trimmed_key = api_key.trim();
    let key_from_user_input = !trimmed_key.is_empty();
    let effective_key = if key_from_user_input {
        trimmed_key.to_string()
    } else {
        crate::llm_config::load_search().api_key
    };
    let cfg = crate::llm_config::SearchConfig {
        provider: provider.trim().to_string(),
        api_key: effective_key,
    };
    info!(
        provider = %cfg.provider,
        key_set = !cfg.api_key.is_empty(),
        key_from_user_input,
        "保存搜索配置"
    );
    crate::llm_config::save_search(&cfg)?;
    // 用新搜索配置重建 LLM 客户端(LLM 客户端持有搜索依赖)。
    // 同 set_llm_config:必须 spawn_blocking 避免 reqwest::blocking::Client::builder().build()
    // 在 async 上下文中析构 tokio runtime 而 panic。
    let new_client = async_runtime::spawn_blocking(move || {
        let llm_cfg = crate::llm_config::load();
        crate::llm_config::create_client(&llm_cfg, &cfg)
    })
    .await
    .map_err(|e| format!("构造 LLM 客户端任务失败: {e}"))?;
    *state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))? = new_client;
    Ok(())
}

// ============== kepubify 持久化配置 ==============

/// 读取持久化的 kepubify 配置。返回 `{ path, enabled }`。
/// `path` 为空表示尚未选择可执行文件。
#[tauri::command]
pub async fn get_kepubify_config() -> Result<serde_json::Value, String> {
    let cfg = crate::llm_config::load_kepubify();
    Ok(serde_json::json!({
        "path": cfg.path,
        "enabled": cfg.enabled,
    }))
}

/// 保存 kepubify 配置。`path` 传空字符串表示清除路径(同时 enabled 自动 false)。
#[tauri::command]
pub async fn set_kepubify_config(path: String, enabled: bool) -> Result<(), String> {
    let trimmed = path.trim().to_string();
    let cfg = crate::llm_config::KepubifyConfig {
        // 路径为空时强制 enabled=false,避免 disk 出现 path="" + enabled=true 的歧义状态
        enabled: !trimmed.is_empty() && enabled,
        path: trimmed,
    };
    info!(
        path_set = !cfg.path.is_empty(),
        enabled = cfg.enabled,
        "保存 kepubify 配置"
    );
    crate::llm_config::save_kepubify(&cfg)
}

// ============== 阶段四 4.6:LLM 元数据建议 ==============

/// 用 LLM 从缓存 source_text 的前约 1 万字推断书名、作者、简介、封面关键词。
///
/// 返回 `{ title?, author?, description?, cover_keywords? }` 或 `null`(无法推断 / 未配置 LLM)。
/// `LlmError::NotConfigured` 静默转 `Ok(null)`,其余错误返回 `Err`。
#[tauri::command]
pub async fn suggest_metadata(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    info!("suggest_metadata 开始");
    // 从缓存取前 1000 字源文本 + 源文件主名(去扩展名)。
    let (sample_text, file_name) = {
        let guard = state
            .pipeline
            .lock()
            .map_err(|e| format!("pipeline 锁中毒: {e}"))?;
        let cached = guard
            .as_ref()
            .ok_or_else(|| "尚未加载文件,请先调用 load_and_analyze".to_string())?;
        let chars: String = cached
            .output
            .source_text
            .chars()
            .take(1_000)
            .collect();
        let name = Path::new(&cached.source_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        (chars, name)
    };

    debug!(
        sample_chars = sample_text.chars().count(),
        file_name = file_name.as_deref().unwrap_or("<none>"),
        "suggest_metadata: 准备调 LLM"
    );

    // 关键:reqwest::blocking 不能在 tokio async 上下文里直接调,必须 spawn_blocking。
    let client = state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))?
        .clone();

    let suggestion_result = async_runtime::spawn_blocking(move || {
        client.suggest_metadata(&sample_text, file_name.as_deref())
    })
    .await
    .map_err(|e| format!("LLM 任务执行失败: {e}"))?;

    use endpoint_core::llm::LlmError;
    match suggestion_result {
        Ok(None) => {
            info!("suggest_metadata 完成: LLM 未返回建议");
            Ok(serde_json::Value::Null)
        }
        Ok(Some(s)) => {
            info!(
                has_title = s.title.is_some(),
                has_author = s.author.is_some(),
                has_description = s.description.is_some(),
                "suggest_metadata 完成"
            );
            Ok(serde_json::json!({
                "title":          s.title,
                "author":         s.author,
                "description":    s.description,
                "cover_keywords": s.cover_keywords,
            }))
        }
        Err(LlmError::NotConfigured) => {
            warn!(
                "suggest_metadata: AppState 持有的是 NoopLlmClient(未配置)。\
                 如已通过 UI 保存配置,请确认确实点了「保存」按钮;直接编辑 config.toml 后需重启应用"
            );
            Ok(serde_json::Value::Null)
        }
        Err(e) => {
            warn!(error = %e, "suggest_metadata LLM 调用失败");
            Err(format!("LLM 调用失败: {e}"))
        }
    }
}

// ============== 阶段四 4.7:LLM 水印仲裁 ==============

/// 获取 span 前后各一行的上下文(跳过空行)。
fn watermark_context(source: &str, start: usize, end: usize) -> (Option<String>, Option<String>) {
    let before = &source[..start.min(source.len())];
    let ctx_before = before
        .rsplit('\n')
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| l.to_string());

    let end_clamped = end.min(source.len());
    let after = &source[end_clamped..];
    let ctx_after = after
        .split('\n')
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| l.to_string());

    (ctx_before, ctx_after)
}

/// 向 LLM 提交一组 suspect 水印候选行,根据裁定结果将 `IsWatermark` 的 verdict 升级为 `Auto`。
///
/// 输入 `spans`:`[{ start: usize, end: usize }]`(字节偏移,与 `WatermarkAnnotation.span` 对齐)。
/// 仅处理 `verdict == "suspect"` 的条目;`auto` 条目传入会被跳过。
///
/// 返回 `{ updated_watermarks, new_cleaning }`:
/// - `updated_watermarks`:verdict 已变为 `auto` 的 `WatermarkAnnotation` JSON 列表。
/// - `new_cleaning`:对应新增的 `CleaningAnnotation` JSON 列表(前端需插入 pipeline.cleaning)。
///
/// `LlmError::NotConfigured` 转 `Err` 并带友好提示。
#[tauri::command]
pub async fn adjudicate_watermarks(
    state: State<'_, AppState>,
    spans: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    info!(spans = spans.len(), "adjudicate_watermarks 开始");
    if spans.is_empty() {
        return Ok(serde_json::json!({ "updated_watermarks": [], "new_cleaning": [] }));
    }

    // 解析 spans
    let target_spans: Vec<(usize, usize)> = spans
        .iter()
        .filter_map(|v| {
            let start = v["start"].as_u64()? as usize;
            let end = v["end"].as_u64()? as usize;
            Some((start, end))
        })
        .collect();

    // 从缓存取候选
    let (candidates, candidate_wm_indices) = {
        let guard = state
            .pipeline
            .lock()
            .map_err(|e| format!("pipeline 锁中毒: {e}"))?;
        let cached = guard
            .as_ref()
            .ok_or_else(|| "尚未加载文件,请先调用 load_and_analyze".to_string())?;
        let source = &cached.output.source_text;
        let watermarks = &cached.output.watermark;

        let mut candidates = Vec::new();
        let mut indices = Vec::new();

        for (start, end) in &target_spans {
            if let Some(idx) = watermarks.iter().position(|w| {
                w.span.start == *start
                    && w.span.end == *end
                    && w.verdict == WatermarkVerdict::Suspect
            }) {
                let line_text = source
                    .get(*start..*end)
                    .unwrap_or("")
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string();
                let (ctx_before, ctx_after) = watermark_context(source, *start, *end);
                candidates.push(endpoint_core::llm::WatermarkCandidate {
                    text: line_text,
                    context_before: ctx_before,
                    context_after: ctx_after,
                });
                indices.push(idx);
            }
        }
        (candidates, indices)
    };

    if candidates.is_empty() {
        return Ok(serde_json::json!({ "updated_watermarks": [], "new_cleaning": [] }));
    }

    // 调 LLM:必须 spawn_blocking 避免 reqwest::blocking 在 async 上下文中析构 panic。
    use endpoint_core::llm::LlmError;
    debug!(candidates = candidates.len(), "提交给 LLM 仲裁");
    let client = state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))?
        .clone();
    let candidates_for_blocking = candidates.clone();
    let llm_result = async_runtime::spawn_blocking(move || {
        client.arbitrate_watermark(&candidates_for_blocking)
    })
    .await
    .map_err(|e| format!("LLM 任务执行失败: {e}"))?;
    let verdicts = match llm_result {
        Ok(v) => v,
        Err(LlmError::NotConfigured) => {
            warn!("adjudicate_watermarks: LLM 未配置");
            return Err("LLM 未配置,请在阶段 4 的 LLM 设置中填写 API key".to_string());
        }
        Err(e) => {
            warn!(error = %e, "adjudicate_watermarks LLM 调用失败");
            return Err(format!("LLM 调用失败: {e}"));
        }
    };

    // 把 IsWatermark 的条目升级 + 写回缓存
    let mut updated_watermarks_json: Vec<serde_json::Value> = Vec::new();
    let mut new_cleaning_json: Vec<serde_json::Value> = Vec::new();

    {
        let mut guard = state
            .pipeline
            .lock()
            .map_err(|e| format!("pipeline 锁中毒: {e}"))?;
        let cached = guard
            .as_mut()
            .ok_or_else(|| "pipeline 缓存已失效".to_string())?;
        let watermarks = &mut cached.output.watermark;
        let cleaning = &mut cached.output.cleaning;

        for (wm_idx, verdict) in candidate_wm_indices.iter().zip(verdicts.iter()) {
            if let AdjudicationVerdict::IsWatermark { reason } = verdict {
                let wm = &mut watermarks[*wm_idx];
                wm.verdict = WatermarkVerdict::Auto;
                wm.signals.push(WatermarkSignal {
                    kind: WatermarkSignalKind::LlmAdjudication,
                    score: 1.0,
                    detail: Some(reason.clone()),
                });

                // 序列化供前端使用
                updated_watermarks_json.push(
                    serde_json::to_value(&*wm)
                        .map_err(|e| format!("序列化水印标注失败: {e}"))?,
                );

                // 新增 cleaning 镜像条目(纯删除,replacement=None)
                let new_ann = CleaningAnnotation {
                    span: wm.span,
                    kind: CleaningKind::WatermarkKeyword,
                    replacement: None,
                };
                new_cleaning_json.push(
                    serde_json::to_value(&new_ann)
                        .map_err(|e| format!("序列化清洗标注失败: {e}"))?,
                );
                // 按 span.start 升序插入
                let pos = cleaning.partition_point(|c| c.span.start <= new_ann.span.start);
                cleaning.insert(pos, new_ann);
            }
        }
    }

    info!(
        upgraded = updated_watermarks_json.len(),
        new_cleaning = new_cleaning_json.len(),
        "adjudicate_watermarks 完成"
    );
    Ok(serde_json::json!({
        "updated_watermarks": updated_watermarks_json,
        "new_cleaning": new_cleaning_json,
    }))
}

// ============== 阶段四 4.8:LLM 规则归纳 + 持久化 ==============

/// 把一组被用户拒绝的水印行发给 LLM,请其归纳一条能匹配该类水印的正则规则。
///
/// `spans`:被拒绝行的字节偏移列表(`[{ start, end }]`)。
/// 返回 [`Rule`] JSON 对象,或 `null`(LLM 无法归纳)。
/// `LlmError::NotConfigured` 返回 `Err`。
#[tauri::command]
pub async fn induce_watermark_rule(
    state: State<'_, AppState>,
    spans: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    info!(spans = spans.len(), "induce_watermark_rule 开始");
    if spans.is_empty() {
        return Ok(serde_json::Value::Null);
    }

    let rejected_lines: Vec<String> = {
        let guard = state
            .pipeline
            .lock()
            .map_err(|e| format!("pipeline 锁中毒: {e}"))?;
        let cached = guard
            .as_ref()
            .ok_or_else(|| "尚未加载文件,请先调用 load_and_analyze".to_string())?;
        let source = &cached.output.source_text;
        spans
            .iter()
            .filter_map(|v| {
                let start = v["start"].as_u64()? as usize;
                let end = v["end"].as_u64()? as usize;
                source
                    .get(start..end)
                    .map(|s| s.trim_matches(['\n', '\r']).trim().to_string())
            })
            .filter(|s| !s.is_empty())
            .collect()
    };

    if rejected_lines.is_empty() {
        return Ok(serde_json::Value::Null);
    }

    use endpoint_core::llm::LlmError;
    debug!(rejected = rejected_lines.len(), "提交给 LLM 归纳规则");
    let client = state
        .llm_client
        .lock()
        .map_err(|e| format!("llm_client 锁中毒: {e}"))?
        .clone();
    let llm_result = async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = rejected_lines.iter().map(|s| s.as_str()).collect();
        client.induce_rule(&refs)
    })
    .await
    .map_err(|e| format!("LLM 任务执行失败: {e}"))?;
    let rule_opt = match llm_result {
        Ok(r) => r,
        Err(LlmError::NotConfigured) => {
            warn!("induce_watermark_rule: LLM 未配置");
            return Err("LLM 未配置,请在阶段 4 的 LLM 设置中填写 API key".to_string());
        }
        Err(e) => {
            warn!(error = %e, "induce_watermark_rule LLM 调用失败");
            return Err(format!("LLM 调用失败: {e}"));
        }
    };

    match rule_opt {
        None => {
            info!("induce_watermark_rule 完成: LLM 未给出规则");
            Ok(serde_json::Value::Null)
        }
        Some(rule) => {
            info!(rule_id = %rule.id, pattern = %rule.pattern, "induce_watermark_rule 完成");
            serde_json::to_value(&rule).map_err(|e| format!("序列化规则失败: {e}"))
        }
    }
}

/// 把前端确认的规则追加(upsert)到 `%APPDATA%\Endpoint\rules.json`。
///
/// 文件不存在时自动创建;存在时按 `rule.id` 替换或追加。
/// 下次 `load_and_analyze` 会自动合并该文件。
#[tauri::command]
pub async fn save_induced_rule(rule: serde_json::Value) -> Result<(), String> {
    let rule: endpoint_core::rules::Rule =
        serde_json::from_value(rule).map_err(|e| format!("规则反序列化失败: {e}"))?;
    info!(rule_id = %rule.id, "保存归纳规则");

    let path = crate::llm_config::user_rules_path()
        .ok_or_else(|| "无法获取配置目录".to_string())?;

    let mut ruleset = if path.exists() {
        endpoint_core::rules::RuleSet::load_from_json(&path)
            .map_err(|e| format!("读取规则文件失败: {e}"))?
    } else {
        endpoint_core::rules::RuleSet::default()
    };

    ruleset.upsert(rule);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }

    ruleset
        .save_to_json(&path)
        .map_err(|e| format!("保存规则失败: {e}"))
}

// ============== 阶段零兼容入口(回归保险,前端不再调用) ==============

#[tauri::command]
pub async fn convert(
    input: String,
    output: String,
    title: String,
    author: String,
    kepubify_path: Option<String>,
) -> Result<String, String> {
    let res = async_runtime::spawn_blocking(move || {
        let metadata = Metadata::new(title, author);
        let input_p = Path::new(&input).to_path_buf();
        let output_p = Path::new(&output).to_path_buf();
        let options = ConvertOptions {
            encoding_override: None,
            rules_path: None,
            kepubify_path: kepubify_path.map(PathBuf::from),
            cancel_token: None,
            watermark: None,
            cleaning: None,
            css_override: None,
            embed_fonts: false,
            font_bytes: None,
        };
        core_convert(&input_p, &output_p, metadata, &options)
    })
    .await
    .map_err(|e| format!("任务调度失败: {e}"))?
    .map_err(|e| format!("{e}"))?;

    Ok(res.display().to_string())
}
