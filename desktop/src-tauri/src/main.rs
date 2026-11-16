// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_key;
mod assets;
mod assistant;
mod beats;
mod generate;
mod music;
mod narration;
mod paths;
mod project;
mod project_assets;
mod render;
mod renders;
mod studio;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(render::RenderState::default())
        .manage(generate::GenerateState::default())
        .setup(|app| {
            assets::allow_public_dir(app);
            studio::allow_root(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            project::read_project_file,
            project::write_project_file,
            project::expand_timeline,
            beats::detect_beats,
            music::music_library,
            music::music_pick,
            narration::listen_narration,
            render::start_render,
            render::cancel_render,
            render::render_defaults,
            render::check_before_render,
            render::reveal_in_finder,
            assets::import_asset,
            assets::asset_root,
            assistant::propose_edit,
            studio::studio_settings,
            studio::set_projects_root,
            studio::list_projects,
            studio::create_project,
            studio::open_project,
            studio::save_project,
            studio::rename_project,
            studio::duplicate_project,
            studio::trash_project,
            studio::import_timeline,
            studio::reveal_path,
            project_assets::list_assets,
            project_assets::import_assets,
            project_assets::trash_asset,
            project_assets::trash_unused_assets,
            renders::list_renders,
            renders::trash_render,
            renders::add_existing_render,
            generate::generate_estimate,
            generate::start_generate,
            generate::cancel_generate
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vimmer Studio");
}
