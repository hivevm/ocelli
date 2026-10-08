//! The desktop host's Tauri commands: each hands its arguments to the `Host` both hosts share
//! and its events to the window, adding nothing to what the command answers (ADR-0037).

use ocelli_commands::session::{Found, Selected, WindowJson};
use ocelli_commands::{Host, Report};
use ocelli_core::port::Write;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State, Window};

/// Shared application state
pub struct AppState {
    /// The repository named on the command line, handed to the web interface once.
    pub startup_repo: Mutex<Option<String>>,
    /// What the window has open: its repository, its load session, and its write.
    pub host: Host,
}

/// The window, where the shared commands send their events.
struct ToWindow(Window);

impl Report for ToWindow {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.0.emit(event, payload);
    }
}

/// The repository named on the command line, if any, the first time it is asked for. The web
/// interface asks once its listeners stand and opens it like any other path, so a load started
/// at startup never emits into a window that is not listening yet.
#[tauri::command]
pub fn startup_repo(state: State<'_, AppState>) -> Option<String> {
    state.startup_repo.lock().unwrap().take()
}

/// Loads a repository by path; the history arrives as events (ADR-0016, ADR-0031).
#[tauri::command]
pub async fn open_repo(
    path: String,
    select: Option<String>,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .host
        .open_repo(PathBuf::from(&path), path, select, ToWindow(window))
}

#[tauri::command(async)]
pub fn read_window(top: usize, rows: usize, state: State<'_, AppState>) -> WindowJson {
    state.host.read_window(top, rows)
}

#[tauri::command(async)]
pub fn select_row(row: usize, state: State<'_, AppState>) -> Option<Selected> {
    state.host.select_row(row)
}

#[tauri::command(async)]
pub fn selection(state: State<'_, AppState>) -> Option<Selected> {
    state.host.selection()
}

#[tauri::command(async)]
pub fn deselect(state: State<'_, AppState>) {
    state.host.deselect()
}

#[tauri::command(async)]
pub fn search_history(query: String, state: State<'_, AppState>) -> Found {
    state.host.search_history(&query)
}

#[tauri::command(async)]
pub fn rows_of(shas: Vec<String>, state: State<'_, AppState>) -> Vec<Option<usize>> {
    state.host.rows_of(&shas)
}

/// Opens the native folder picker and returns the chosen path.
/// Loading starts right away, so the caller needs no follow-up call.
#[tauri::command]
pub async fn browse_repo(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app
        .dialog()
        .file()
        .set_title("Select Git Repository Folder")
        .blocking_pick_folder();

    if let Some(folder) = path {
        let path_str = folder.to_string();
        // Load the chosen folder directly
        let _ = open_repo(path_str.clone(), None, window, state).await;
        Ok(Some(path_str))
    } else {
        Ok(None)
    }
}

/// Loads the files a commit changed and sends them as a `commit-files` event.
#[tauri::command]
pub async fn select_commit(
    sha: String,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.host.select_commit(sha, &ToWindow(window)).await
}

#[tauri::command]
pub fn write_discards(write: Write) -> Option<String> {
    Host::write_discards(&write)
}

#[tauri::command]
pub async fn perform_write(
    write: Write,
    confirmed: bool,
    window: Window,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .host
        .perform_write(write, confirmed, ToWindow(window))
        .await
}

#[tauri::command(async)]
pub fn cancel_write(state: State<'_, AppState>) {
    state.host.cancel_write();
}

#[tauri::command]
pub async fn get_file_diff(
    sha: String,
    file_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state.host.get_file_diff(sha, file_path).await
}
