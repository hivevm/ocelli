use ocelli_core::graph;
use ocelli_core::model::MAX_COMMITS;
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State, Window};

/// Shared application state
pub struct AppState {
    pub current_repo: Mutex<Option<PathBuf>>,
    /// The repository named on the command line, handed to the web interface once.
    pub startup_repo: Mutex<Option<String>>,
}

/// The repository named on the command line, if any, the first time it is asked for. The web
/// interface asks once its listeners stand and opens it like any other path, so a load started
/// at startup never emits into a window that is not listening yet.
#[tauri::command]
pub fn startup_repo(state: State<'_, AppState>) -> Option<String> {
    state.startup_repo.lock().unwrap().take()
}

/// Loads a repository by path and emits graph data to the frontend.
/// Returns as soon as the load is spawned; the graph arrives as a `load-graph` event.
#[tauri::command]
pub async fn open_repo(
    path: String,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let path = PathBuf::from(&path);
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    // Update shared state
    *state.current_repo.lock().unwrap() = Some(path.clone());

    // Emit status + repo label immediately, before the background load starts
    let _ = window.emit("set-status", "Loading...");
    let _ = window.emit("set-repo-label", path.to_string_lossy().to_string());

    // Spawn the load in the background so this command never blocks the UI
    let window_clone = window.clone();
    tokio::spawn(async move {
        // Progress channel: sends messages from git loading to the window
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);
        let w2 = window_clone.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let _ = w2.emit("set-status", msg);
            }
        });

        match load_and_layout(path, tx).await {
            Ok((graph_json, refs_json)) => {
                let node_count = count_nodes_from_json(&graph_json);
                let _ = window_clone.emit("load-graph", graph_json);
                // The sidebar's refs come from the same read as the history, so
                // the two never describe different states of the repository.
                let _ = window_clone.emit("load-refs", refs_json);
                let status = loaded_status(node_count);
                // The same line on standard error, so that a script driving the application,
                // scripts/measure-memory.sh among them, can tell that the graph has loaded.
                eprintln!("{status}");
                let _ = window_clone.emit("set-status", status);
            }
            Err(e) => {
                let _ = window_clone.emit("set-status", format!("Error: {}", e));
            }
        }
    });

    Ok(())
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
        let _ = open_repo(path_str.clone(), window, state).await;
        Ok(Some(path_str))
    } else {
        Ok(None)
    }
}

/// Receives commit selection from JS, loads changed files and emits them back.
#[tauri::command]
pub async fn select_commit(
    sha: String,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let repo_path = match state.current_repo.lock().unwrap().clone() {
        Some(p) => p,
        None => return Err("No repository open".into()),
    };

    let sha_clone = sha.clone();
    let files =
        tokio::task::spawn_blocking(move || GitReader.load_commit_files(&repo_path, &sha_clone))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;

    let json = serde_json::to_string(&files).map_err(|e| e.to_string())?;
    let _ = window.emit("commit-files", json);
    Ok(())
}

/// Returns the diff lines for a single file in a commit.
#[tauri::command]
pub async fn get_file_diff(
    sha: String,
    file_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo_path = match state.current_repo.lock().unwrap().clone() {
        Some(p) => p,
        None => return Err("No repository open".into()),
    };

    let lines =
        tokio::task::spawn_blocking(move || GitReader.load_file_diff(&repo_path, &sha, &file_path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;

    serde_json::to_string(&lines).map_err(|e| e.to_string())
}

/// Loads a repository, builds the graph layout, and reads the refs the sidebar
/// lists (ADR-0026). Returns both as JSON: the graph and the refs describe one
/// and the same read of the repository.
/// Runs git2 operations on a blocking thread pool.
async fn load_and_layout(
    path: PathBuf,
    progress_tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(String, String), anyhow::Error> {
    // git2 is not async-safe so run on blocking thread pool
    let refs_path = path.clone();
    let commits = tokio::task::spawn_blocking(move || {
        // Runs on a blocking thread, so the progress channel may block too.
        let mut progress = |msg: String| {
            let _ = progress_tx.blocking_send(msg);
        };
        GitReader.load_commits(&path, &mut progress)
    })
    .await??;

    let repo_refs = tokio::task::spawn_blocking(move || GitReader.load_refs(&refs_path)).await??;

    let graph = graph::build_graph(commits);
    let graph_json = serde_json::to_string(&graph)?;
    let refs_json = serde_json::to_string(&repo_refs)?;
    Ok((graph_json, refs_json))
}

/// The status line after a load. At the commit limit the load was (or may have been) truncated, so
/// it says so instead of presenting the cut-off as the whole history. It starts with the number of
/// commits and ` commits`, the shape `scripts/measure-memory.sh` reads from standard error to tell
/// that the graph has loaded.
fn loaded_status(node_count: usize) -> String {
    if node_count >= MAX_COMMITS {
        format!(
            "{} commits — commit limit reached, older history not shown",
            node_count
        )
    } else {
        format!("{} commits", node_count)
    }
}

/// Counts nodes in a serialized JSON graph (for status message).
fn count_nodes_from_json(json: &str) -> usize {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json)
        && let Some(nodes) = val.get("nodes").and_then(|n| n.as_array())
    {
        return nodes.len();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::loaded_status;
    use ocelli_core::model::MAX_COMMITS;

    #[test]
    fn the_status_below_the_limit_names_the_commits() {
        assert_eq!(loaded_status(42), "42 commits");
    }

    #[test]
    fn the_status_at_the_limit_says_older_history_is_cut_and_keeps_the_shape() {
        let line = loaded_status(MAX_COMMITS);
        assert_eq!(
            line,
            format!("{MAX_COMMITS} commits — commit limit reached, older history not shown")
        );
    }
}
