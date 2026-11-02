mod common;
mod modules;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let root = modules::settings::root(app.handle())?;
            app.manage(modules::map_tiles::MapTiles::new(root.clone())?);
            app.manage(Arc::new(
                modules::downloader::service::DownloadService::load(root)?,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            modules::sar_search::sar_search,
            modules::sar_search::cancel_sar_search,
            modules::settings::load_settings,
            modules::settings::save_settings,
            modules::settings::choose_output_dir,
            modules::settings::open_output_dir,
            modules::settings::export_results,
            modules::area::list_admin_regions,
            modules::boundary::choose_boundary_file,
            modules::map_tiles::load_map_tile,
            modules::map_tiles::load_map_resource,
            modules::map_tiles::cancel_map_tile,
            modules::downloader::service::add_downloads,
            modules::downloader::service::list_downloads,
            modules::downloader::service::start_download,
            modules::downloader::service::pause_download,
            modules::downloader::service::remove_download,
            modules::downloader::service::remove_downloads,
            modules::downloader::cleanup::preview_download_cleanup,
            modules::downloader::cleanup::execute_download_cleanup,
            modules::downloader::storage::probe_download_space,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
