#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod brave_client;
mod commands;
mod llm_config;
mod openai_client;
mod state;

use state::AppState;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() {
    init_tracing();
    tracing::info!("endpoint-app 启动");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // 阶段二主入口
            commands::load_and_analyze,
            commands::build_epub,
            commands::cancel_task,
            // 文件对话框
            commands::pick_input_file,
            commands::pick_output_file,
            commands::pick_executable_file,
            commands::pick_cover_file,
            commands::pick_font_file,
            // 阶段四 4.2:CSS 主题
            commands::list_themes,
            commands::load_theme,
            // 阶段四 4.3:文字封面
            commands::generate_text_cover,
            // 阶段四 4.5:LLM 配置
            commands::get_llm_config,
            commands::set_llm_config,
            commands::set_search_config,
            // kepubify 持久化配置
            commands::get_kepubify_config,
            commands::set_kepubify_config,
            // 阶段四 4.6:LLM 元数据建议
            commands::suggest_metadata,
            // 阶段四 4.7:LLM 水印仲裁
            commands::adjudicate_watermarks,
            // 阶段四 4.8:LLM 规则归纳 + 持久化
            commands::induce_watermark_rule,
            commands::save_induced_rule,
            // 阶段零兼容入口(回归保险)
            commands::convert,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}

/// 装配 tracing 订阅器。
///
/// 默认级别 `info`；通过环境变量 `RUST_LOG` 可临时调整（例：
/// `RUST_LOG=endpoint_core::watermark=debug,endpoint_app=debug`）。
/// 当前只输出到 stdout；后续若需要文件/UI 面板，加 Layer 即可，业务代码不动。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,endpoint_core=info,endpoint_app=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_ansi(true),
        )
        .init();
}
