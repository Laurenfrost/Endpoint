#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::convert,
            commands::pick_input_file,
            commands::pick_output_file,
            commands::pick_executable_file,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
