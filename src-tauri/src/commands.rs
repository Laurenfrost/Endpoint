//! Tauri 命令:**薄**桥接层。
//!
//! 此层只做参数转换、把核心库调用放到 blocking 线程池、把错误转成前端可消费的字符串。
//! 所有业务逻辑都在 `endpoint-core` 里。
//!
//! 文件选择对话框走 Rust 端的 `tauri-plugin-dialog`,避免前端依赖 plugin 的 JS(纯静态
//! HTML 没有 bundler,`withGlobalTauri` 只暴露 core API,plugin JS 默认不可用)。

use std::path::PathBuf;

use endpoint_core::{convert as core_convert, ConvertOptions, Metadata};
use tauri::async_runtime;
use tauri_plugin_dialog::{DialogExt, FilePath};

fn file_path_to_string(fp: FilePath) -> Option<String> {
    match fp {
        FilePath::Path(p) => Some(p.display().to_string()),
        FilePath::Url(u) => Some(u.to_string()),
    }
}

#[tauri::command]
pub async fn pick_input_file(app: tauri::AppHandle) -> Option<String> {
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
    app: tauri::AppHandle,
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
pub async fn pick_executable_file(app: tauri::AppHandle) -> Option<String> {
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
        let input_p = PathBuf::from(input);
        let output_p = PathBuf::from(output);
        // 阶段一:桥接层暂不暴露编码覆盖/规则文件入口,使用默认 ConvertOptions(自动探测 + 内置规则)。
        // 阶段二的界面会改用 run_pipeline + build_epub_from 拆两步走,届时再加这些参数。
        let options = ConvertOptions {
            encoding_override: None,
            rules_path: None,
            kepubify_path: kepubify_path.map(PathBuf::from),
        };
        core_convert(&input_p, &output_p, metadata, &options)
    })
    .await
    .map_err(|e| format!("任务调度失败: {e}"))?
    .map_err(|e| format!("{e}"))?;

    Ok(res.display().to_string())
}
