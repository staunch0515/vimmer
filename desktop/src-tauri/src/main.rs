// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod project;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            project::read_project_file,
            project::write_project_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vimmer Studio");
}
