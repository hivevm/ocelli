//! One WebSocket per browser tab, carrying the desktop host's commands and events with the same
//! names and payloads, each connection a load session of its own (ADR-0035).
//!
//! A message from the browser is a command, `{"id", "cmd", "args"}`, the arguments as `invoke`
//! passes them; the server answers `{"id", "ok"}` with the value the Tauri command returns, or
//! `{"id", "err"}` with the reason it rejects with. An event is `{"event", "payload"}`. Every
//! event, and the failure of every command that reached the repository, leaves the server through
//! `scrub`, so that no path outside the server root leaves it (ADR-0037). A write runs only where
//! the server was started with `--allow-writes`, through the shared `Host` as in the desktop
//! host, taking turns with the other tabs' writes to its repository (ADR-0035).

pub mod scrub;

use crate::Limits;
use crate::root::Root;
use crate::turns::Turns;
use axum::extract::ws::{Message, WebSocket};
use ocelli_commands::{Host, Report};
use ocelli_core::port::Write;
use scrub::{READ_FAILED, outbound, scrub, scrub_write};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinSet;

/// The reason every write is refused with where the server was started without `--allow-writes`
/// (ADR-0035).
pub const WRITES_REFUSED: &str =
    "The remote server only reads: it writes only when started with --allow-writes";

/// What a tab is told, as the server's own `repo-written` event, once another tab wrote to the
/// repository it has open, which it then loads again (ADR-0035).
pub const WRITTEN_ELSEWHERE: &str =
    "Another tab wrote to this repository: it is shown as it is now";

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
/// the last of what holds it, the connection, a command, or a load, has let it go (ADR-0035).
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
    /// `--allow-writes` (ADR-0035).
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
/// in time, then stops its load and drops its load session (ADR-0035). Its writes take turns with
/// the other tabs' in `writes`, none where the server refuses every write (ADR-0035).
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
    // Every command that reaches the repository checks it again (ADR-0037).
    let host = Host::guarded(Arc::new(move |path| guard_root.verify(path)));
    // Set once the connection ends: a write still waiting for its turn then gives it up.
    let gone = Arc::new(AtomicBool::new(false));
    let number = writes.as_ref().map_or(0, |turns| turns.tabs().tab());
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
    // (ADR-0035).
    tab.host.cancel_write();
    gone.store(true, Ordering::Release);
    if let Some(turns) = writes {
        turns.tabs().forget(number);
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
struct MergePath {
    path: String,
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
/// same `Host` (ADR-0035).
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
                        .and_then(|path| ocelli_git::refs::common_dir(&path));
                    let to = tab.to.clone();
                    let tell = Arc::new(move || to.report("repo-written", WRITTEN_ELSEWHERE));
                    turns.tabs().watch(tab.number, common, tell, &tab.gone);
                }
                Ok(())
            })
            .await?;
            value(())
        }
        // The counterpart of the desktop's folder dialog `browse_repo`, which the server does not
        // answer: a command of the same name never answers otherwise (ADR-0035, ADR-0037).
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
        "get_merge_file" => {
            let MergePath { path } = args(a)?;
            value(host.get_merge_file(path).await.map_err(scrub)?)
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
    pub(super) struct Tree(pub(super) PathBuf);

    impl Tree {
        pub(super) fn new(name: &str) -> Tree {
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
