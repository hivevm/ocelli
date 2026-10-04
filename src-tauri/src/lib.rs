mod commands;

use commands::AppState;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            current_repo: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_repo,
            commands::browse_repo,
            commands::select_commit,
            commands::get_file_diff,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
