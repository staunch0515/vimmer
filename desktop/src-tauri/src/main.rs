// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod project;
mod render;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(render::RenderState::default())
        .invoke_handler(tauri::generate_handler![
            project::read_project_file,
            project::write_project_file,
            render::start_render,
            render::cancel_render,
            render::reveal_in_finder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vimmer Studio");
}
