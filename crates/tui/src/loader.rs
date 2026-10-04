//! Background git access. git2 never runs on the thread that draws: the
//! repository load and every per-commit read happen on worker threads, and
//! their results come back to the event loop as [`Msg`] values (ADR-0028, the
//! terminal counterpart of ADR-0019).

use ocelli_core::graph;
use ocelli_core::model::{DiffLine, FileChange, Graph, RepoRefs};
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

/// A result arriving at the event loop.
pub enum Msg {
    Progress(String),
    Loaded(Result<(Graph, RepoRefs), String>),
    Files {
        sha: String,
        files: Result<Vec<FileChange>, String>,
    },
    Diff {
        sha: String,
        path: String,
        lines: Result<Vec<DiffLine>, String>,
    },
}

/// A read the event loop asks the worker for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Files { sha: String },
    Diff { sha: String, path: String },
}

/// Loads history, refs and layout on a thread of its own. The graph and the
/// refs come from the same read, so they never describe different states.
pub fn spawn_load(path: PathBuf, tx: Sender<Msg>) {
    thread::spawn(move || {
        let reader = GitReader;
        let relay = tx.clone();
        let mut progress = move |msg: String| {
            let _ = relay.send(Msg::Progress(msg));
        };

        let result = (|| -> anyhow::Result<(Graph, RepoRefs)> {
            let commits = reader.load_commits(&path, &mut progress)?;
            let repo_refs = reader.load_refs(&path)?;
            Ok((graph::build_graph(commits), repo_refs))
        })()
        .map_err(|e| e.to_string());
        let _ = tx.send(Msg::Loaded(result));
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
