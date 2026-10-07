//! Background git access. git2 never runs on the thread that draws: the
//! repository load and every per-commit read happen on worker threads, and
//! their results come back to the event loop as [`Msg`] values (ADR-0028, the
//! terminal counterpart of ADR-0019). A write runs on a thread of its own too
//! (ADR-0033).

use ocelli_core::load::{self, Event};
use ocelli_core::model::{DiffLine, FileChange};
use ocelli_core::port::{self, Confirmation, RepositoryReader, Write};
use ocelli_git::{GitReader, GitWriter};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

/// A result arriving at the event loop.
pub enum Msg {
    /// How the load of the history goes (ADR-0016).
    Load(Event),
    Files {
        sha: String,
        files: Result<Vec<FileChange>, String>,
    },
    Diff {
        sha: String,
        path: String,
        lines: Result<Vec<DiffLine>, String>,
    },
    /// A write done, with the commit it made where it made one, or refused with nothing changed.
    Written(Result<Option<String>, String>),
}

/// A read the event loop asks the worker for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Files { sha: String },
    Diff { sha: String, path: String },
}

/// Loads the history on a thread of its own (ADR-0016), laid out there, so the
/// event loop only takes in rows laid out. The load stops once the event loop
/// has dropped the receiver, as when another repository is opened.
pub fn spawn_load(path: PathBuf, tx: Sender<Msg>) {
    thread::spawn(move || load::load(&GitReader, &path, &mut |e| tx.send(Msg::Load(e)).is_ok()));
}

/// Performs a write on a thread of its own: unlike reads, a write is never
/// collapsed into a newer one. The core refuses one that discards work
/// without the developer's confirmation (ADR-0034).
pub fn spawn_write(path: PathBuf, write: Write, confirmation: Confirmation, tx: Sender<Msg>) {
    thread::spawn(move || {
        let result =
            port::perform(&GitWriter, &path, &write, confirmation).map_err(|e| e.to_string());
        let _ = tx.send(Msg::Written(result));
    });
}

/// Starts the worker for per-commit reads. Requests that queue up while one is
/// running are collapsed to the newest of each kind: scrolling quickly through
/// the graph must not leave a backlog of file lists nobody looks at any more.
pub fn spawn_worker(path: PathBuf, tx: Sender<Msg>) -> Sender<Request> {
    let (req_tx, req_rx) = channel::<Request>();
    thread::spawn(move || {
        let reader = GitReader;
        while let Some((files, diff)) = next_batch(&req_rx) {
            if let Some(sha) = files {
                let files = reader
                    .load_commit_files(&path, &sha)
                    .map_err(|e| e.to_string());
                if tx.send(Msg::Files { sha, files }).is_err() {
                    return;
                }
            }
            if let Some((sha, file)) = diff {
                let lines = reader
                    .load_file_diff(&path, &sha, &file)
                    .map_err(|e| e.to_string());
                let msg = Msg::Diff {
                    sha,
                    path: file,
                    lines,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    });
    req_tx
}

type Batch = (Option<String>, Option<(String, String)>);

/// Blocks for one request, then drains whatever else is queued, keeping only
/// the newest request of each kind. `None` once the event loop has gone.
fn next_batch(rx: &Receiver<Request>) -> Option<Batch> {
    let mut batch: Batch = (None, None);
    let mut take = |req| match req {
        Request::Files { sha } => batch.0 = Some(sha),
        Request::Diff { sha, path } => batch.1 = Some((sha, path)),
    };
    take(rx.recv().ok()?);
    while let Ok(req) = rx.try_recv() {
        take(req);
    }
    Some(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verifies: G-13
    fn queued_requests_collapse_to_the_newest_of_each_kind() {
        let (tx, rx) = channel();
        for sha in ["a", "b", "c"] {
            tx.send(Request::Files { sha: sha.into() }).unwrap();
        }
        tx.send(Request::Diff {
            sha: "c".into(),
            path: "f".into(),
        })
        .unwrap();
        assert_eq!(
            next_batch(&rx),
            Some((Some("c".into()), Some(("c".into(), "f".into()))))
        );
        drop(tx);
        assert_eq!(next_batch(&rx), None);
    }
}
