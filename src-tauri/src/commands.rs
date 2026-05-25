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
use serde::Serialize;
use tauri::{async_runtime, AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

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
        let bytes = std::fs::read(&path_for_blocking)
            .map_err(|e| format!("读取文件失败({path_for_blocking}): {e}"))?;
        let options = ConvertOptions {
            encoding_override,
            rules_path: None,
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
        .map_err(|e| format!("任务调度失败: {e}"))?
        .map_err(|e| format!("解析失败: {e}"))?;

    let dto = serde_json::to_value(&pipeline)
        .map_err(|e| format!("序列化 PipelineOutput 失败: {e}"))?;

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
        .map_err(|e| format!("任务调度失败: {e}"))?
        .map_err(|e| format!("构建失败: {e}"))?;

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
    }
    Ok(())
}

// ============== 阶段四 4.2:CSS 主题 ==============

/// 列出可用主题名称列表(resource themes/ 目录下所有 .css 文件的 stem)。
/// 顺序固定:standard → classic → highcontrast;其余按字母序追加。
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

    // 把内置三主题置前,其余按字母序
    let priority = ["standard", "classic", "highcontrast"];
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
