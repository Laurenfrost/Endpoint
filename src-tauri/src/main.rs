#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod llm_config;
mod openai_client;
mod state;

use state::AppState;

fn main() {
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
