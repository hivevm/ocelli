//! One WebSocket per browser tab, carrying the desktop host's commands and events with the same
//! names and payloads, each connection a load session of its own (ADR-0037).
//!
//! A message from the browser is a command, `{"id", "cmd", "args"}`, the arguments as `invoke`
//! passes them; the server answers `{"id", "ok"}` with the value the Tauri command returns, or
//! `{"id", "err"}` with the reason it rejects with. An event is `{"event", "payload"}`. No path
//! outside the server root leaves the server (ADR-0039): the paths of the sidebar's rows are sent
//! relative to it, or empty where they lie outside, and what git says is replaced. A write runs
//! only where the server was started with `--allow-writes`, through the shared `Host` as in the
//! desktop host, taking turns with the other tabs' writes to its repository (ADR-0037).

use crate::Limits;
use crate::root::Root;
use crate::turns::Turns;
use axum::extract::ws::{Message, WebSocket};
use ocelli_commands::{Host, HostError, Report};
use ocelli_core::port::Write;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinSet;

/// The reason every write is refused with where the server was started without `--allow-writes`
/// (ADR-0037).
pub const WRITES_REFUSED: &str =
    "The remote server only reads: it writes only when started with --allow-writes";

/// What the failure of a write the git adapter reported says in the browser, its refusals
/// included: its words may name a path on the server (ADR-0039).
pub const WRITE_FAILED: &str =
    "The write was refused or failed: reload to see the repository as it is";

/// What a tab is told, as the server's own `repo-written` event, once another tab wrote to the
/// repository it has open, which it then loads again (ADR-0037).
pub const WRITTEN_ELSEWHERE: &str =
    "Another tab wrote to this repository: it is shown as it is now";

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

/// The failure of a command that reached the repository as the browser may read it, decided by
/// its kind: Ocelli's own refusals and that no repository is open as they are, and anything git
/// said, which may name a path on the server, as `READ_FAILED` (ADR-0039).
pub fn scrub(error: HostError) -> String {
    match error {
        HostError::NoRepository | HostError::Refused(_) => error.to_string(),
        HostError::Git(_) => READ_FAILED.into(),
    }
}

/// The failure of a write as the browser may read it: `scrub`, with what git said, or the git
/// adapter's refusal, which its type does not tell from it, as `WRITE_FAILED`.
pub fn scrub_write(error: HostError) -> String {
    match error {
        HostError::Git(_) => WRITE_FAILED.into(),
        error => scrub(error),
    }
}

/// A transfer's progress as the browser may read it: a word that is an address keeps its scheme,
/// host, and path but not the user name and password it may carry, and a word that names a path
/// on the server, absolute or with a backslash, reads `<path>`. The git adapter's progress names
/// the remote and counts objects; what a repository's configuration names is untrusted, so what
/// may reach the words is scrubbed here (ADR-0039, ADR-0037).
pub fn scrub_progress(text: &str) -> String {
    let scrubbed: Vec<String> = text.split(' ').map(scrub_word).collect();
    scrubbed.join(" ")
}

fn scrub_word(word: &str) -> String {
    if let Some((scheme, rest)) = word.split_once("://") {
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        // A local address's path is a path on the server.
        let path = if scheme.eq_ignore_ascii_case("file") {
            "<path>"
        } else {
            path
        };
        return format!("{scheme}://{host}{path}");
    }
    let bare = word.trim_start_matches(['"', '\'', '(', '[', '<', '`']);
    let drive =
        bare.len() > 2 && bare.as_bytes()[0].is_ascii_alphabetic() && bare[1..].starts_with(":/");
    if bare.starts_with(['/', '~']) || bare.contains('\\') || drive {
        "<path>".into()
    } else {
        word.into()
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
        "write-progress" => {
            if let Some(text) = payload.as_str() {
                *payload = Value::String(scrub_progress(text));
            }
        }
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
    /// The turns of the server's writes, where the tab may write: the server was started with
    /// `--allow-writes` (ADR-0037).
    writes: Option<Turns>,
    /// The tab's number among the turns' tabs.
    number: u64,
    /// Set once the connection has ended.
    gone: Arc<AtomicBool>,
    /// Held while the tab's open repository is registered with the turns.
    registering: std::sync::Mutex<()>,
    /// Whether the web interface has asked for the repository to open at its start.
    started: AtomicBool,
}

/// Serves one connection until the browser closes it, or until it does not read what it is sent
/// in time, then stops its load and drops its load session (ADR-0037). Its writes take turns with
/// the other tabs' in `writes`, none where the server refuses every write (ADR-0037).
pub async fn serve(
    mut socket: WebSocket,
    root: Arc<Root>,
    limits: Limits,
    sessions: Sessions,
    writes: Option<Turns>,
) {
    let (tx, mut rx) = mpsc::channel::<String>(limits.queue);
    let overflowed = Arc::new(Notify::new());
    let guard_root = Arc::clone(&root);
    // Every command that reaches the repository checks it again (ADR-0039).
    let host = Host::guarded(Arc::new(move |path| guard_root.verify(path)));
    // Set once the connection ends: a write still waiting for its turn then gives it up.
    let gone = Arc::new(AtomicBool::new(false));
    let number = writes.as_ref().map_or(0, Turns::tab);
    let tab = Arc::new(Tab {
        writes: writes.clone(),
        number,
        gone: Arc::clone(&gone),
        registering: std::sync::Mutex::new(()),
        host: match writes.clone() {
            Some(turns) => {
                let gone = Arc::clone(&gone);
                host.taking_turns(Arc::new(move |path| turns.of(path, &gone, number)))
            }
            None => host,
        },
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
    // The commands that run, aborted when the connection ends, so that a closed tab holds no
    // task: its slot is free for another, which must not find its work still queued.
    let mut running = JoinSet::new();
    // A connection that died without a close, its browser gone or its network, sends nothing,
    // not even the answer to a ping: past `idle` it is closed and its tab's slot freed.
    let mut pings =
        tokio::time::interval_at(tokio::time::Instant::now() + limits.ping, limits.ping);
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = overflowed.notified() => break,
            Some(_) = running.join_next(), if !running.is_empty() => {}
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
                running.spawn(async move {
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
    // A local write already writing finishes, as a write in the desktop host does when its
    // window closes; a fetch, or a pull while it fetches, stops, as `cancel_write` stops it, so
    // that a tab gone holds no repository's turn for a transfer nobody waits for; a push, which
    // cannot be stopped, finishes. One still waiting for its turn gives it up unwritten
    // (ADR-0037).
    tab.host.cancel_write();
    gone.store(true, Ordering::Release);
    if let Some(turns) = writes {
        turns.forget(number);
        turns.wake();
    }
    running.abort_all();
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

/// The arguments of `perform_write`, as the desktop host's command takes them.
#[derive(Deserialize)]
struct Perform {
    write: Write,
    confirmed: bool,
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
                    .map_err(scrub)?;
                if let Some(turns) = &tab.writes {
                    // The repository the tab shows now, read under the lock: of two opens of one
                    // tab at once, the one that registers last reads the repository the later
                    // open left, whichever finished first.
                    let _one = tab.registering.lock().unwrap();
                    let common = tab
                        .host
                        .current_repo()
                        .and_then(|path| ocelli_git::confine::common_dir(&path));
                    let to = tab.to.clone();
                    let tell = Arc::new(move || to.report("repo-written", WRITTEN_ELSEWHERE));
                    turns.watch(tab.number, common, tell, &tab.gone);
                }
                Ok(())
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
        "perform_write" => {
            // Refused before its arguments are read: a server without the switch writes nothing.
            if tab.writes.is_none() {
                return Err(WRITES_REFUSED.into());
            }
            let Perform { write, confirmed } = args(a)?;
            let made = host.perform_write(write, confirmed, tab.to.clone()).await;
            value(made.map_err(scrub_write)?)
        }
        "cancel_write" => {
            host.cancel_write();
            value(())
        }
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
        // What git says is replaced whatever its words, Ocelli's own included.
        for said in [
            "failed to resolve path '/nonexistent/x': No such file or directory".to_string(),
            "could not open C:\\Users\\a\\repo".to_string(),
            crate::root::refused("app"),
        ] {
            assert_eq!(scrub(HostError::Git(said.clone())), READ_FAILED, "{said}");
        }
        // A refusal reaches the browser with its reason, whatever its words.
        for reason in [
            crate::root::refused("app"),
            crate::root::too_large("app"),
            "any reason at all".to_string(),
        ] {
            assert_eq!(scrub(HostError::Refused(reason.clone())), reason);
        }
        assert_eq!(
            scrub(HostError::NoRepository),
            HostError::NoRepository.to_string()
        );
        // A write's failure from git, the git adapter's refusal among them, says that it failed;
        // Ocelli's own refusals pass as they do for a read.
        for said in [
            "the checked-out commit is no longer the one shown: reload to see it as it is",
            "failed to write /srv/repo/.git/index.lock: Permission denied",
        ] {
            assert_eq!(scrub_write(HostError::Git(said.into())), WRITE_FAILED);
        }
        assert_eq!(
            scrub_write(HostError::Refused("A write is still running".into())),
            "A write is still running"
        );
        assert_eq!(
            scrub_write(HostError::NoRepository),
            HostError::NoRepository.to_string()
        );
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
    // Verifies: ADR-0037, ADR-0039, Q-7
    fn a_transfer_s_progress_names_no_credential_and_no_path_on_the_server() {
        for (said, read) in [
            (
                "Fetching origin: 3 of 5 objects",
                "Fetching origin: 3 of 5 objects",
            ),
            ("Pushing a: 1 of 2 objects", "Pushing a: 1 of 2 objects"),
            (
                "Fetching https://ada:SECRET@example.org/r.git: authenticating",
                "Fetching https://example.org/r.git: authenticating",
            ),
            (
                "Fetching ssh://ada@host:22: the server is preparing",
                "Fetching ssh://host:22: the server is preparing",
            ),
            (
                "Fetching file:///srv/root/remote.git: 1 of 1 objects",
                "Fetching file://<path> 1 of 1 objects",
            ),
            (
                "from /srv/root/remote.git ('/home/ada/x') C:/Users/ada C:\\Users\\ada ~/r",
                "from <path> <path> <path> <path> <path>",
            ),
        ] {
            assert_eq!(scrub_progress(said), read, "{said}");
        }
        let tree = Tree::new("progress");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let mut payload = json!("Fetching https://u:p@h/r: /srv/x");
        outbound(&root, "write-progress", &mut payload);
        assert_eq!(payload, "Fetching https://h/r: <path>");
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
