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
use tauri::{async_runtime, AppHandle, Emitter, State};
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
) -> Result<serde_json::Value, String> {
    let task_id = state.next_task_id("load");
    let cancel_flag = state.register_cancel(&task_id);

    // 阶段三 v2:把前端传的 cleaning_config(JSON 对象)反序列化为 CleaningConfig。
    // 反序列化失败时返回错误,不静默 fallback——避免用户改了配置但实际未生效。
    let cleaning_cfg = match cleaning_config {
        Some(v) => Some(
            serde_json::from_value::<endpoint_core::cleaning::CleaningConfig>(v)
                .map_err(|e| format!("cleaning_config 反序列化失败: {e}"))?,
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
            // 阶段三 3.5(推迟到阶段四)之前用 default;前端暂无入口调阈值。
            watermark: None,
            // 阶段三 v2 新增:前端策略面板的勾选状态
            cleaning: cleaning_cfg,
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
#[tauri::command]
pub async fn build_epub(
    app: AppHandle,
    state: State<'_, AppState>,
    output_path: String,
    title: String,
    author: String,
    kepubify_path: Option<String>,
) -> Result<String, String> {
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
        build_epub_from(
            &pipeline_for_build,
            &output_owned,
            kepubify_owned.as_deref(),
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
        };
        core_convert(&input_p, &output_p, metadata, &options)
    })
    .await
    .map_err(|e| format!("任务调度失败: {e}"))?
    .map_err(|e| format!("{e}"))?;

    Ok(res.display().to_string())
}
