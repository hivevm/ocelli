//! One WebSocket per browser tab, carrying the desktop host's commands and events with the same
//! names and payloads, each connection a load session of its own (ADR-0037).
//!
//! A message from the browser is a command, `{"id", "cmd", "args"}`, the arguments as `invoke`
//! passes them; the server answers `{"id", "ok"}` with the value the Tauri command returns, or
//! `{"id", "err"}` with the reason it rejects with. An event is `{"event", "payload"}`. No path
//! outside the server root leaves the server (ADR-0039): the paths of the sidebar's rows are sent
//! relative to it, or empty where they lie outside, and what git says is replaced.

use crate::Limits;
use crate::root::Root;
use axum::extract::ws::{Message, WebSocket};
use ocelli_commands::{Host, Report};
use ocelli_core::port::Write;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore, mpsc};

/// The reason every write is refused with until the remote server's second feature spec brings
/// writes (ADR-0037).
pub const WRITES_REFUSED: &str =
    "The remote server only reads: writing is not offered in the browser";

/// What a reason that names a path on the server says instead.
pub const READ_FAILED: &str = "Reading the repository failed";

/// The largest message a browser may send: a command and its arguments.
pub const MAX_MESSAGE: usize = 1 << 20;

/// The most commands of one tab answered at once; the next is read once one is answered.
const COMMANDS_AT_ONCE: usize = 16;

#[derive(Deserialize)]
struct Command {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Value,
}

/// The reason of a command that reached the repository as the browser may read it: Ocelli's own
/// refusals as they are, and anything git said, which may name a path on the server, replaced
/// by `READ_FAILED` (ADR-0039).
pub fn scrub(reason: String) -> String {
    if crate::root::is_refusal(&reason) || reason == "No repository open" {
        reason
    } else {
        READ_FAILED.into()
    }
}

/// A status line with what git said, after a load's error, replaced by `READ_FAILED`.
fn scrub_status(status: &mut Value) {
    let Some(text) = status.as_str() else {
        return;
    };
    let scrubbed = if text.starts_with("Error: ") {
        format!("Error: {READ_FAILED}")
    } else if let Some((rows, _)) = text.split_once(" — loading stopped: ") {
        format!("{rows} — loading stopped: {READ_FAILED}")
    } else {
        return;
    };
    *status = Value::String(scrubbed);
}

/// The payload of `event` with no path outside the server root: the sidebar's working trees,
/// submodules, and parent row carry their path relative to it, or none where they lie outside,
/// which shows the row without its switch, the web interface saying why (ADR-0039); a status
/// line says what git said only as `READ_FAILED`.
pub fn outbound(root: &Root, event: &str, payload: &mut Value) {
    let relative = |value: &mut Value| {
        if let Some(path) = value.as_str() {
            let inside = (!path.is_empty())
                .then(|| root.relative(Path::new(path)))
                .flatten();
            *value = Value::String(inside.unwrap_or_default());
        }
    };
    match event {
        "load-refs" => {
            let refs = &mut payload["refs"];
            for worktree in refs["worktrees"].as_array_mut().into_iter().flatten() {
                relative(&mut worktree["path"]);
            }
            for submodule in refs["submodules"].as_array_mut().into_iter().flatten() {
                relative(&mut submodule["workdir"]);
            }
            if refs["parent"].is_object() {
                relative(&mut refs["parent"]["path"]);
            }
        }
        "set-status" => scrub_status(payload),
        "history-grew" => scrub_status(&mut payload["status"]),
        _ => {}
    }
}

/// The socket, where the shared commands send their events and answers: a queue of a bounded
/// length, which a tab that does not read overflows, and the connection then closes.
#[derive(Clone)]
pub struct ToSocket {
    tx: mpsc::Sender<String>,
    overflowed: Arc<Notify>,
    root: Arc<Root>,
    /// Counts the tab's load session while anything can still report to it: the tab, a command
    /// it runs, or its load.
    _held: Arc<Held>,
}

/// How many tabs' load sessions the server holds: each counted from its connection's start until
/// the last of what holds it, the connection, a command, or a load, has let it go (ADR-0037).
#[derive(Clone, Default)]
pub struct Sessions(Arc<AtomicUsize>);

impl Sessions {
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    fn hold(&self) -> Held {
        self.0.fetch_add(1, Ordering::AcqRel);
        Held(Arc::clone(&self.0))
    }
}

/// One load session counted by `Sessions`, until it is dropped.
struct Held(Arc<AtomicUsize>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ToSocket {
    fn send(&self, text: String) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send(text) {
            self.overflowed.notify_one();
        }
        // A closed socket has nobody to tell.
    }
}

impl Report for ToSocket {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let mut payload = serde_json::to_value(payload).unwrap_or(Value::Null);
        outbound(&self.root, event, &mut payload);
        self.send(json!({ "event": event, "payload": payload }).to_string());
    }
}

/// What one connection holds: what the desktop host holds for its window.
struct Tab {
    host: Host,
    root: Arc<Root>,
    to: ToSocket,
    /// Whether the web interface has asked for the repository to open at its start.
    started: AtomicBool,
}

/// Serves one connection until the browser closes it, or until it does not read what it is sent
/// in time, then stops its load and drops its load session (ADR-0037).
pub async fn serve(mut socket: WebSocket, root: Arc<Root>, limits: Limits, sessions: Sessions) {
    let (tx, mut rx) = mpsc::channel::<String>(limits.queue);
    let overflowed = Arc::new(Notify::new());
    let guard_root = Arc::clone(&root);
    let tab = Arc::new(Tab {
        // Every command that reaches the repository checks it again (ADR-0039).
        host: Host::guarded(Arc::new(move |path| guard_root.verify(path))),
        to: ToSocket {
            tx,
            overflowed: Arc::clone(&overflowed),
            root: Arc::clone(&root),
            _held: Arc::new(sessions.hold()),
        },
        root,
        started: AtomicBool::new(false),
    });
    let commands = Arc::new(Semaphore::new(COMMANDS_AT_ONCE));
    // A connection that died without a close, its browser gone or its network, sends nothing,
    // not even the answer to a ping: past `idle` it is closed and its tab's slot freed.
    let mut pings =
        tokio::time::interval_at(tokio::time::Instant::now() + limits.ping, limits.ping);
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = overflowed.notified() => break,
            _ = pings.tick() => {
                if heard.elapsed() >= limits.idle {
                    break;
                }
                let sent = tokio::time::timeout(
                    limits.send_wait,
                    socket.send(Message::Ping(Default::default())),
                );
                if !matches!(sent.await, Ok(Ok(()))) {
                    break;
                }
            }
            outgoing = rx.recv() => {
                let Some(text) = outgoing else { break };
                let sent = tokio::time::timeout(
                    limits.send_wait,
                    socket.send(Message::Text(text.into())),
                );
                if !matches!(sent.await, Ok(Ok(()))) {
                    break;
                }
            }
            incoming = socket.recv() => {
                heard = tokio::time::Instant::now();
                let command = match incoming {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Command>(&text),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    // A close, a binary message, or a broken connection ends it.
                    _ => break,
                };
                let Ok(command) = command else {
                    // Not a command: the browser does not speak this protocol.
                    break;
                };
                let Ok(permit) = Arc::clone(&commands).acquire_owned().await else {
                    break;
                };
                let tab = Arc::clone(&tab);
                tokio::spawn(async move {
                    let answer = match answer(&tab, &command.cmd, command.args).await {
                        Ok(value) => json!({ "id": command.id, "ok": value }),
                        Err(reason) => json!({ "id": command.id, "err": reason }),
                    };
                    tab.to.send(answer.to_string());
                    drop(permit);
                });
            }
        }
    }
    tab.host.stop();
}

/// `work` on the blocking pool, for what reads the file system.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| READ_FAILED.to_string())?
}

fn args<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("Invalid arguments: {e}"))
}

fn value<T: Serialize>(answer: T) -> Result<Value, String> {
    serde_json::to_value(answer).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct OpenRepo {
    path: String,
    #[serde(default)]
    select: Option<String>,
}

#[derive(Deserialize)]
struct Folder {
    path: String,
}

#[derive(Deserialize)]
struct ReadWindow {
    top: usize,
    rows: usize,
}

#[derive(Deserialize)]
struct Row {
    row: usize,
}

#[derive(Deserialize)]
struct Query {
    query: String,
}

#[derive(Deserialize)]
struct Shas {
    shas: Vec<String>,
}

#[derive(Deserialize)]
struct Sha {
    sha: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDiff {
    sha: String,
    file_path: String,
}

#[derive(Deserialize)]
struct Discards {
    write: Write,
}

/// The answer to one command, as the desktop host's command of the same name gives it, from the
/// same `Host` (ADR-0037).
async fn answer(tab: &Arc<Tab>, cmd: &str, a: Value) -> Result<Value, String> {
    let host = &tab.host;
    match cmd {
        "startup_repo" => {
            let first = !tab.started.swap(true, Ordering::AcqRel);
            let root = Arc::clone(&tab.root);
            // The check walks the repository's git directory: off the async runtime.
            let open = blocking(move || Ok(first && root.is_repository())).await?;
            value(open.then_some("."))
        }
        "open_repo" => {
            let OpenRepo { path, select } = args(a)?;
            let tab = Arc::clone(tab);
            blocking(move || {
                let opened = tab.root.open(&path)?;
                tab.host
                    .open_repo(opened.path, opened.label, select, tab.to.clone())
            })
            .await?;
            value(())
        }
        // The counterpart of the desktop's folder dialog `browse_repo`, which the server does not
        // answer: a command of the same name never answers otherwise (ADR-0037, ADR-0039).
        "list_folders" => {
            let Folder { path } = args(a)?;
            let root = Arc::clone(&tab.root);
            value(blocking(move || root.list(&path)).await?)
        }
        "read_window" => {
            let ReadWindow { top, rows } = args(a)?;
            value(host.read_window(top, rows))
        }
        "select_row" => {
            let Row { row } = args(a)?;
            value(host.select_row(row))
        }
        "selection" => value(host.selection()),
        "deselect" => {
            host.deselect();
            value(())
        }
        "search_history" => {
            let Query { query } = args(a)?;
            value(host.search_history(&query))
        }
        "rows_of" => {
            let Shas { shas } = args(a)?;
            value(host.rows_of(&shas))
        }
        "select_commit" => {
            let Sha { sha } = args(a)?;
            value(host.select_commit(sha, &tab.to).await.map_err(scrub)?)
        }
        "get_file_diff" => {
            let FileDiff { sha, file_path } = args(a)?;
            value(host.get_file_diff(sha, file_path).await.map_err(scrub)?)
        }
        "write_discards" => {
            let Discards { write } = args(a)?;
            value(Host::write_discards(&write))
        }
        "perform_write" => Err(WRITES_REFUSED.into()),
        "cancel_write" => value(()),
        _ => Err(format!("Unknown command: {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::path::PathBuf;

    /// A server root with a folder inside it, and a folder beside it.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir =
                std::env::temp_dir().join(format!("ocelli-socket-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for folder in ["root/wt", "outside"] {
                std::fs::create_dir_all(dir.join(folder)).unwrap();
            }
            Tree(dir)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    // Verifies: ADR-0039
    fn the_sidebar_s_paths_leave_the_server_relative_to_the_root_or_not_at_all() {
        let tree = Tree::new("refs");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let inside = tree.0.join("root/wt").to_string_lossy().into_owned();
        let outside = tree.0.join("outside").to_string_lossy().into_owned();
        let mut payload = json!({ "refs": {
            "worktrees": [{ "path": inside }, { "path": outside }],
            "submodules": [{ "workdir": inside }, { "workdir": "" }, { "workdir": outside }],
            "parent": { "path": outside },
        }});
        outbound(&root, "load-refs", &mut payload);
        let refs = &payload["refs"];
        assert_eq!(refs["worktrees"][0]["path"], "wt");
        assert_eq!(refs["worktrees"][1]["path"], "");
        assert_eq!(refs["submodules"][0]["workdir"], "wt");
        assert_eq!(refs["submodules"][1]["workdir"], "");
        assert_eq!(refs["submodules"][2]["workdir"], "");
        assert_eq!(
            refs["parent"],
            json!({ "path": "" }),
            "a parent outside shows without its switch"
        );
        let mut parent = json!({ "refs": { "parent": { "name": "wt", "path": inside } } });
        outbound(&root, "load-refs", &mut parent);
        assert_eq!(parent["refs"]["parent"]["path"], "wt");
        let mut none = json!({ "refs": { "worktrees": [], "submodules": [], "parent": null } });
        outbound(&root, "load-refs", &mut none);
        assert_eq!(none["refs"]["parent"], Value::Null);
    }

    #[test]
    // Verifies: ADR-0039
    fn what_git_says_reaches_the_browser_only_as_reading_failed() {
        for said in [
            "failed to resolve path '/nonexistent/x': No such file or directory",
            "revspec 'c' not found; class=Reference (4); code=NotFound (-3)",
            "could not open C:\\Users\\a\\repo",
        ] {
            assert_eq!(scrub(said.to_string()), READ_FAILED, "{said}");
        }
        for ours in [
            "No repository open",
            "app is no repository below the server root",
        ] {
            assert_eq!(scrub(ours.to_string()), ours);
        }
        let tree = Tree::new("status");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let mut status = json!("Error: failed to resolve path '/srv/x'");
        outbound(&root, "set-status", &mut status);
        assert_eq!(status, format!("Error: {READ_FAILED}"));
        let mut grew = json!({ "rows": 3, "status": "3 commits — loading stopped: object /srv/x" });
        outbound(&root, "history-grew", &mut grew);
        assert_eq!(
            grew["status"],
            format!("3 commits — loading stopped: {READ_FAILED}")
        );
        for plain in ["3 commits", "Loaded 3000 commits...", "Loading..."] {
            let mut status = json!(plain);
            outbound(&root, "set-status", &mut status);
            assert_eq!(status, plain);
        }
    }

    #[test]
    fn a_tab_whose_queue_overflows_is_told_to_close() {
        let tree = Tree::new("queue");
        let (tx, _rx) = mpsc::channel(2);
        let overflowed = Arc::new(Notify::new());
        let to = ToSocket {
            tx,
            overflowed: Arc::clone(&overflowed),
            root: Arc::new(Root::new(&tree.0.join("root")).unwrap()),
            _held: Arc::new(Sessions::default().hold()),
        };
        to.report("set-status", "one");
        to.report("set-status", "two");
        assert!(
            overflowed.notified().now_or_never().is_none(),
            "room for two"
        );
        to.report("set-status", "three");
        assert!(overflowed.notified().now_or_never().is_some());
    }
}
