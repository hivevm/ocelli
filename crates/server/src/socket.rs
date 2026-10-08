//! One WebSocket per browser tab, carrying the desktop host's commands and events with the same
//! names and payloads, each connection a load session of its own (ADR-0037).
//!
//! A message from the browser is a command, `{"id", "cmd", "args"}`, the arguments as `invoke`
//! passes them; the server answers `{"id", "ok"}` with the value the Tauri command returns, or
//! `{"id", "err"}` with the reason it rejects with. An event is `{"event", "payload"}`.

use crate::root::Root;
use axum::extract::ws::{Message, WebSocket};
use ocelli_commands::{Host, Report};
use ocelli_core::port::Write;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// The reason every write is refused with until the remote server's second feature spec brings
/// writes (ADR-0037).
pub const WRITES_REFUSED: &str =
    "The remote server only reads: writing is not offered in the browser";

/// The reason the desktop's folder dialog has no counterpart yet: the web interface opens the
/// repository at the server root (ADR-0039).
pub const NO_FOLDER_DIALOG: &str =
    "The browser opens the repository at the server root; choosing another folder is not offered";

/// The largest message a browser may send: a command and its arguments.
pub const MAX_MESSAGE: usize = 1 << 20;

#[derive(Deserialize)]
struct Command {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Value,
}

/// The socket, where the shared commands send their events.
#[derive(Clone)]
pub struct ToSocket(mpsc::UnboundedSender<String>);

impl Report for ToSocket {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let message = json!({ "event": event, "payload": payload });
        // A closed socket has nobody to tell.
        let _ = self.0.send(message.to_string());
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

/// Serves one connection until the browser closes it, then stops its load and drops its load
/// session (ADR-0037).
pub async fn serve(mut socket: WebSocket, root: Arc<Root>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let tab = Arc::new(Tab {
        host: Host::default(),
        root,
        to: ToSocket(tx),
        started: AtomicBool::new(false),
    });
    loop {
        tokio::select! {
            outgoing = rx.recv() => {
                let Some(text) = outgoing else { break };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
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
                let tab = Arc::clone(&tab);
                tokio::spawn(async move {
                    let answer = match answer(&tab, &command.cmd, command.args).await {
                        Ok(value) => json!({ "id": command.id, "ok": value }),
                        Err(reason) => json!({ "id": command.id, "err": reason }),
                    };
                    let _ = tab.to.0.send(answer.to_string());
                });
            }
        }
    }
    tab.host.stop();
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
async fn answer(tab: &Tab, cmd: &str, a: Value) -> Result<Value, String> {
    let host = &tab.host;
    match cmd {
        "startup_repo" => {
            let first = !tab.started.swap(true, Ordering::AcqRel);
            value((first && tab.root.is_repository()).then_some("."))
        }
        "open_repo" => {
            let OpenRepo { path, select } = args(a)?;
            let opened = tab.root.open(&path)?;
            value(host.open_repo(opened.path, opened.label, select, tab.to.clone())?)
        }
        "browse_repo" => Err(NO_FOLDER_DIALOG.into()),
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
            value(host.select_commit(sha, &tab.to).await?)
        }
        "get_file_diff" => {
            let FileDiff { sha, file_path } = args(a)?;
            value(host.get_file_diff(sha, file_path).await?)
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
