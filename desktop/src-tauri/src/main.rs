// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod project;
mod render;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(render::RenderState::default())
        .setup(|app| {
            assets::allow_public_dir(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            project::read_project_file,
            project::write_project_file,
            render::start_render,
            render::cancel_render,
            render::reveal_in_finder,
            assets::import_asset,
            assets::asset_root
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vimmer Studio");
}
