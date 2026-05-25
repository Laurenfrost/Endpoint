#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
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
            // 阶段零兼容入口(回归保险)
            commands::convert,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
