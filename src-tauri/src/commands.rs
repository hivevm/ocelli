use crate::session::{Found, Selected, Session, WindowJson};
use ocelli_core::load::{self, Event};
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State, Window};

/// Shared application state
pub struct AppState {
    pub current_repo: Mutex<Option<PathBuf>>,
    /// The repository named on the command line, handed to the web interface once.
    pub startup_repo: Mutex<Option<String>>,
    /// The history of the newest load (ADR-0031).
    pub session: Arc<Session>,
}

/// The repository named on the command line, if any, the first time it is asked for. The web
/// interface asks once its listeners stand and opens it like any other path, so a load started
/// at startup never emits into a window that is not listening yet.
#[tauri::command]
pub fn startup_repo(state: State<'_, AppState>) -> Option<String> {
    state.startup_repo.lock().unwrap().take()
}

/// Loads a repository by path. Returns as soon as the load is spawned; the history arrives chunk
/// by chunk, each announced by a `history-grew` event, and the refs as a `load-refs` event right
/// after the first chunk, read with it so the two describe the same state (ADR-0016, ADR-0031).
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

    // git2 is not async-safe, so the load runs on the blocking pool and never blocks the UI.
    // The load session begins here, before the load is spawned, so loads begin in the order
    // they were asked for and the newest is the one shown.
    let session = Arc::clone(&state.session);
    let generation = session.begin();
    tokio::task::spawn_blocking(move || load(&GitReader, &path, &session, generation, &window));
    Ok(())
}

/// Where a load reports how far it has come: the window, or a recorder in tests.
pub trait Report {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S);
}

impl Report for Window {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.emit(event, payload);
    }
}

/// Loads the history of `path` into `session` (ADR-0016), laying each chunk out on this thread,
/// and tells the window how far it has come. A newer load refuses this one's chunks, which stops
/// its walk, and keeps it from writing the status line.
fn load(
    reader: &impl RepositoryReader,
    path: &Path,
    session: &Session,
    generation: u64,
    window: &impl Report,
) {
    load::load(reader, path, &mut |event| match event {
        Event::Progress(msg) => {
            if session.is_current(generation) {
                window.report("set-status", msg);
            }
            true
        }
        Event::Chunk(chunk) => {
            let Some(growth) = session.append(generation, chunk) else {
                return false;
            };
            window.report("history-grew", &growth);
            window.report("set-status", &growth.status);
            true
        }
        Event::Refs(refs) => {
            let current = session.is_current(generation);
            if current {
                window.report("load-refs", &refs);
            }
            current
        }
        Event::Ended(result) => {
            // The layout is gone by now; what the load freed goes back before it says so.
            release_freed_memory();
            let complete = result.is_ok();
            let error = result.as_ref().err().cloned();
            match (session.finish(generation, result), error) {
                (Some(growth), _) => {
                    window.report("history-grew", &growth);
                    if complete {
                        // The same line on standard error, so that a script driving the
                        // application, scripts/measure-memory.sh among them, can tell that the
                        // history has loaded.
                        eprintln!("{}", growth.status);
                    }
                    window.report("set-status", growth.status);
                }
                (None, Some(e)) if session.is_current(generation) => {
                    window.report("set-status", format!("Error: {e}"));
                }
                (None, _) => {}
            }
            false
        }
    });
}

/// Hands the memory the load has freed back to the system. The walk and the layout free most of
/// what they allocate once the load ends, but glibc keeps freed memory with the process, where Q-3
/// counts it; other allocators hand it back on their own.
#[allow(unsafe_code)]
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: malloc_trim takes no pointer and only returns free pages of glibc's own heaps
        // to the system; it is safe to call from any thread at any time.
        unsafe {
            malloc_trim(0);
        }
    }
}

/// Rows `top..top + rows` of the history, with the edges crossing them and, while a commit is
/// selected, which of them belong to its lineage.
#[tauri::command(async)]
pub fn read_window(top: usize, rows: usize, state: State<'_, AppState>) -> WindowJson {
    state.session.window(top, rows)
}

/// Selects the commit in `row`: its lineage lights up in the windows read after.
#[tauri::command(async)]
pub fn select_row(row: usize, state: State<'_, AppState>) -> Option<Selected> {
    state.session.select(row)
}

/// The selected commit again, its relatives as far as they have loaded by now.
#[tauri::command(async)]
pub fn selection(state: State<'_, AppState>) -> Option<Selected> {
    state.session.selection()
}

#[tauri::command(async)]
pub fn deselect(state: State<'_, AppState>) {
    state.session.deselect()
}

/// The rows matching `query`, newest first, with the nodes of the first ones the results panel
/// lists.
#[tauri::command(async)]
pub fn search_history(query: String, state: State<'_, AppState>) -> Found {
    state.session.search(&query, SEARCH_HITS)
}

/// The most matches the results panel lists; every match is reachable by cycling through them.
const SEARCH_HITS: usize = 200;

/// The row of each SHA, null for one not laid out yet.
#[tauri::command(async)]
pub fn rows_of(shas: Vec<String>, state: State<'_, AppState>) -> Vec<Option<usize>> {
    state.session.rows_of(&shas)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::three;
    use ocelli_core::model::{CommitData, DiffLine, FileChange, RepoRefs};
    use std::cell::RefCell;

    /// Records what a load reports, one line per event.
    #[derive(Default)]
    struct Recorder(RefCell<Vec<String>>);

    impl Report for Recorder {
        fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
            let value = serde_json::to_value(payload).unwrap();
            let line = match event {
                "history-grew" => format!(
                    "grew {} rows{}",
                    value["rows"],
                    if value["loading"] == true {
                        ", loading"
                    } else {
                        ""
                    }
                ),
                "load-refs" => "refs".to_string(),
                _ => format!("{event} {}", value.as_str().unwrap_or_default()),
            };
            self.0.borrow_mut().push(line);
        }
    }

    /// A repository whose walk yields `chunks` and then fails if `walk_fails`, whose refs fail if
    /// `refs_fail`, and which begins a newer load on the session before the chunk `newer_at`.
    #[derive(Default)]
    struct Fake {
        chunks: Vec<Vec<CommitData>>,
        walk_fails: bool,
        refs_fail: bool,
        newer_at: Option<(usize, Arc<Session>)>,
    }

    impl RepositoryReader for Fake {
        fn load_chunks(
            &self,
            _: &Path,
            progress: &mut dyn FnMut(String),
            on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
        ) -> anyhow::Result<()> {
            progress("walking".into());
            for (i, chunk) in self.chunks.iter().enumerate() {
                if let Some((at, session)) = &self.newer_at
                    && i == *at
                {
                    session.begin();
                }
                if !on_chunk(chunk.clone()) {
                    return Ok(());
                }
                progress("walking on".into());
            }
            if self.walk_fails {
                anyhow::bail!("walk failed");
            }
            Ok(())
        }

        fn load_main_tip(&self, _: &Path) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        fn load_refs(&self, _: &Path) -> anyhow::Result<RepoRefs> {
            if self.refs_fail {
                anyhow::bail!("refs failed");
            }
            Ok(RepoRefs::default())
        }

        fn load_commit_files(&self, _: &Path, _: &str) -> anyhow::Result<Vec<FileChange>> {
            unimplemented!()
        }

        fn load_file_diff(&self, _: &Path, _: &str, _: &str) -> anyhow::Result<Vec<DiffLine>> {
            unimplemented!()
        }
    }

    fn run(fake: &Fake, session: &Session) -> Vec<String> {
        let recorder = Recorder::default();
        load(
            fake,
            Path::new("/repo"),
            session,
            session.begin(),
            &recorder,
        );
        recorder.0.into_inner()
    }

    #[test]
    // Verifies: ADR-0016, ADR-0031, G-13
    fn a_load_reports_each_chunk_the_refs_after_the_first_and_the_end() {
        let fake = Fake {
            chunks: vec![three(9), three(6)],
            ..Fake::default()
        };
        assert_eq!(
            run(&fake, &Session::default()),
            [
                "set-status walking",
                "grew 3 rows, loading",
                "set-status 3 commits in 1 chunk — loading older history…",
                "refs",
                "grew 6 rows, loading",
                "set-status 6 commits in 2 chunks — loading older history…",
                "grew 6 rows",
                "set-status 6 commits",
            ]
        );
    }

    #[test]
    // Verifies: ADR-0016, G-13
    fn a_walk_failing_after_the_first_chunk_keeps_it_and_says_so() {
        let fake = Fake {
            chunks: vec![three(9)],
            walk_fails: true,
            ..Fake::default()
        };
        let reported = run(&fake, &Session::default());
        assert_eq!(
            reported.last().unwrap(),
            "set-status 3 commits — loading stopped: walk failed"
        );
    }

    #[test]
    // Verifies: ADR-0016, G-13
    fn a_walk_failing_before_the_first_chunk_is_an_error() {
        let fake = Fake {
            walk_fails: true,
            ..Fake::default()
        };
        assert_eq!(
            run(&fake, &Session::default()),
            ["set-status walking", "set-status Error: walk failed"]
        );
    }

    #[test]
    // Verifies: ADR-0016
    fn refs_failing_stop_the_load_and_say_so_last() {
        let fake = Fake {
            chunks: vec![three(9), three(6)],
            refs_fail: true,
            ..Fake::default()
        };
        let reported = run(&fake, &Session::default());
        assert_eq!(
            reported.last().unwrap(),
            "set-status 3 commits — loading stopped: refs failed"
        );
        assert!(
            !reported.iter().any(|r| r.starts_with("grew 6")),
            "{reported:?}"
        );
    }

    #[test]
    // Verifies: ADR-0016
    fn a_load_superseded_before_its_first_chunk_reports_nothing() {
        let session = Session::default();
        let older = session.begin();
        session.begin();
        let fake = Fake {
            chunks: vec![three(9)],
            walk_fails: true,
            ..Fake::default()
        };
        let recorder = Recorder::default();
        load(&fake, Path::new("/repo"), &session, older, &recorder);
        assert_eq!(recorder.0.into_inner(), Vec::<String>::new());
    }

    #[test]
    // Verifies: ADR-0016
    fn a_newer_load_stops_the_walk_of_the_older() {
        let session = Arc::new(Session::default());
        let fake = Fake {
            chunks: vec![three(9), three(6), three(3)],
            newer_at: Some((1, Arc::clone(&session))),
            ..Fake::default()
        };
        let reported = run(&fake, &session);
        assert_eq!(
            reported.last().unwrap(),
            "refs",
            "nothing after the newer load began"
        );
    }
}
