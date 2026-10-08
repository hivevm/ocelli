//! The commands and events of a host of the web interface, once for both hosts (ADR-0037): the
//! desktop host adapts them to Tauri's commands and events, the remote server to its WebSocket,
//! and neither adds anything to what a command answers. A `Host` holds what one window, or one
//! browser tab, has open: its repository, its load session, and its write in flight. Events go to
//! a `Report`, the window or the socket.

pub mod session;

use ocelli_core::load::{self, Event};
use ocelli_core::model::RepoRefs;
use ocelli_core::port::{self, Confirmation, RepositoryReader, Write};
use ocelli_core::transfer::Transfers;
use ocelli_git::{GitReader, GitWriter};
use serde::Serialize;
use session::{Found, Selected, Session, WindowJson};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where a host tells the web interface what happens: the window, the socket, or a recorder in
/// tests. The event names and payloads are the desktop host's (ADR-0037).
pub trait Report {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S);
}

/// What a host checks a repository's path against before every command that reaches it: the
/// remote server's server root (ADR-0039). It answers why it refuses.
pub type Guard = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// What one window or tab has open.
#[derive(Default)]
pub struct Host {
    current_repo: Mutex<Option<PathBuf>>,
    /// Checked before every command that reaches the repository; none in the desktop host.
    guard: Option<Guard>,
    /// The history of the newest load (ADR-0031).
    session: Arc<Session>,
    /// The flag that stops the write that runs, replaced for each write.
    cancel: Mutex<Arc<AtomicBool>>,
    /// Whether a write runs: the host takes one at a time.
    writing: AtomicBool,
}

impl Host {
    /// A host that checks the repository's path with `guard` before the load and before every
    /// command that reads or writes it, so that what changed on disk since it was opened is
    /// checked again (ADR-0039).
    pub fn guarded(guard: Guard) -> Host {
        Host {
            guard: Some(guard),
            ..Host::default()
        }
    }

    fn check(&self, path: &Path) -> Result<(), String> {
        checked(&self.guard, path)
    }

    /// Loads the repository at `path`, labelled `label` for the web interface. Returns as soon as
    /// the load is spawned; the history arrives chunk by chunk, each announced by a
    /// `history-grew` event, and the refs as a `load-refs` event right after the first chunk,
    /// read with it so the two describe the same state (ADR-0016, ADR-0031). `select` names the
    /// commit a write left selected, which a `history-grew` event names the row of once it is
    /// laid out (ADR-0033).
    ///
    /// A guarded host checks `path` here, reading the file system: a caller on an async runtime
    /// calls this on its blocking pool.
    pub fn open_repo<R: Report + Send + 'static>(
        &self,
        path: PathBuf,
        label: String,
        select: Option<String>,
        to: R,
    ) -> Result<(), String> {
        if !path.exists() {
            return Err(format!("Path does not exist: {label}"));
        }
        self.check(&path)?;
        *self.current_repo.lock().unwrap() = Some(path.clone());

        // Status and label first, before the background load starts.
        to.report("set-status", "Loading...");
        to.report("set-repo-label", label);

        // git2 is not async-safe, so the load runs on the blocking pool and never blocks the UI.
        // The load session begins here, before the load is spawned, so loads begin in the order
        // they were asked for and the newest is the one shown.
        let session = Arc::clone(&self.session);
        let generation = session.begin(select);
        tokio::task::spawn_blocking(move || load(&GitReader, &path, &session, generation, &to));
        Ok(())
    }

    /// Stops the load that runs, for a window or a tab that closes: a newer generation refuses
    /// its next chunk, which stops its walk (ADR-0037).
    pub fn stop(&self) {
        self.session.begin(None);
    }

    /// Rows `top..top + rows` of the history, with the edges crossing them and, while a commit is
    /// selected, which of them belong to its lineage.
    pub fn read_window(&self, top: usize, rows: usize) -> WindowJson {
        self.session.window(top, rows)
    }

    /// Selects the commit in `row`: its lineage lights up in the windows read after.
    pub fn select_row(&self, row: usize) -> Option<Selected> {
        self.session.select(row)
    }

    /// The selected commit again, its relatives as far as they have loaded by now.
    pub fn selection(&self) -> Option<Selected> {
        self.session.selection()
    }

    pub fn deselect(&self) {
        self.session.deselect()
    }

    /// The rows matching `query`, newest first, with the nodes of the first ones the results
    /// panel lists.
    pub fn search_history(&self, query: &str) -> Found {
        self.session.search(query, SEARCH_HITS)
    }

    /// The row of each SHA, null for one not laid out yet.
    pub fn rows_of(&self, shas: &[String]) -> Vec<Option<usize>> {
        self.session.rows_of(shas)
    }

    /// The open repository and the guard to check it with, on the blocking pool with the read
    /// it guards, since the check reads the file system.
    fn repo(&self) -> Result<(PathBuf, Option<Guard>), String> {
        let path = self
            .current_repo
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "No repository open".to_string())?;
        Ok((path, self.guard.clone()))
    }

    /// Loads the files `sha` changed and sends them as a `commit-files` event.
    pub async fn select_commit<R: Report>(&self, sha: String, to: &R) -> Result<(), String> {
        let (repo_path, guard) = self.repo()?;
        let files = tokio::task::spawn_blocking(move || {
            checked(&guard, &repo_path)?;
            GitReader
                .load_commit_files(&repo_path, &sha)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        let json = serde_json::to_string(&files).map_err(|e| e.to_string())?;
        to.report("commit-files", json);
        Ok(())
    }

    /// The diff lines of one file in a commit, as JSON.
    pub async fn get_file_diff(&self, sha: String, file_path: String) -> Result<String, String> {
        let (repo_path, guard) = self.repo()?;
        let lines = tokio::task::spawn_blocking(move || {
            checked(&guard, &repo_path)?;
            GitReader
                .load_file_diff(&repo_path, &sha, &file_path)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        serde_json::to_string(&lines).map_err(|e| e.to_string())
    }

    /// What `write` discards, in the core's words for the question the web interface puts
    /// before it, or null for a write that discards nothing (ADR-0034).
    pub fn write_discards(write: &Write) -> Option<String> {
        write.discards()
    }

    /// Performs `write` (G-7): a write that discards work only where the developer `confirmed`
    /// it in the web interface's dialog, which the core decides (ADR-0034). A write refused
    /// changes nothing and says why; the web interface reloads after a write done, the commit
    /// selected again, or the commit the write made, which this returns (ADR-0033).
    pub async fn perform_write<R: Report + Send + 'static>(
        &self,
        write: Write,
        confirmed: bool,
        to: R,
    ) -> Result<Option<String>, String> {
        let confirmation = if confirmed {
            Confirmation::Given
        } else {
            Confirmation::Absent
        };
        let (repo_path, guard) = self.repo()?;
        // One write at a time, so that a cancel reaches the write it was meant for.
        if self.writing.swap(true, Ordering::AcqRel) {
            return Err("A write is still running".into());
        }
        // A transfer reports how far it has come as `write-progress` events, at most every
        // tenth of a second, and stops once `cancel_write` sets the flag this write was handed
        // (ADR-0035).
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().unwrap() = Arc::clone(&cancel);
        let written = tokio::task::spawn_blocking(move || {
            checked(&guard, &repo_path)?;
            let mut told: Option<std::time::Instant> = None;
            let mut progress = |p: &str| {
                if told.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    told = Some(std::time::Instant::now());
                    to.report("write-progress", p);
                }
                !cancel.load(Ordering::Relaxed)
            };
            port::perform(&GitWriter, &repo_path, &write, confirmation, &mut progress)
                .map_err(|e| e.to_string())
        })
        .await;
        self.writing.store(false, Ordering::Release);
        written.map_err(|e| e.to_string())?
    }

    /// Stops the write that runs, where it reports its progress.
    pub fn cancel_write(&self) {
        self.cancel.lock().unwrap().store(true, Ordering::Relaxed);
    }
}

/// `path` checked with `guard`, where the host has one.
fn checked(guard: &Option<Guard>, path: &Path) -> Result<(), String> {
    guard.as_ref().map_or(Ok(()), |guard| guard(path))
}

/// The most matches the results panel lists; every match is reachable by cycling through them.
const SEARCH_HITS: usize = 200;

/// How often a write's progress reaches the web interface at most.
const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// The `load-refs` payload: the refs the sidebar lists, and the transfers the toolbar offers as
/// the core decides them from the same refs, which the web interface draws without restating
/// (ADR-0036).
#[derive(Debug, Clone, Serialize)]
pub struct LoadedRefs<'a> {
    pub refs: &'a RepoRefs,
    pub transfers: Transfers,
}

impl<'a> LoadedRefs<'a> {
    pub fn of(refs: &'a RepoRefs) -> Self {
        LoadedRefs {
            refs,
            transfers: refs.transfers(),
        }
    }
}

/// Loads the history of `path` into `session` (ADR-0016), laying each chunk out on this thread,
/// and tells the web interface how far it has come. A newer load refuses this one's chunks,
/// which stops its walk, and keeps it from writing the status line.
fn load(
    reader: &impl RepositoryReader,
    path: &Path,
    session: &Session,
    generation: u64,
    to: &impl Report,
) {
    load::load(reader, path, &mut |event| match event {
        Event::Progress(msg) => {
            if session.is_current(generation) {
                to.report("set-status", msg);
            }
            true
        }
        Event::Chunk(chunk) => {
            let Some(growth) = session.append(generation, chunk) else {
                return false;
            };
            to.report("history-grew", &growth);
            to.report("set-status", &growth.status);
            true
        }
        Event::Refs(refs) => {
            let refs: Arc<RepoRefs> = Arc::from(refs);
            let current = session.keep_refs(generation, Arc::clone(&refs));
            if current {
                to.report("load-refs", LoadedRefs::of(&refs));
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
                    to.report("history-grew", &growth);
                    if complete {
                        // The same line on standard error, so that a script driving the
                        // application, scripts/measure-memory.sh among them, can tell that the
                        // history has loaded.
                        eprintln!("{}", growth.status);
                    }
                    to.report("set-status", growth.status);
                }
                (None, Some(e)) if session.is_current(generation) => {
                    to.report("set-status", format!("Error: {e}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::three;
    use ocelli_core::model::{CommitData, DiffLine, FileChange, LocalBranch};
    use std::cell::RefCell;

    /// Records what a load reports, one line per event, and the payloads of `load-refs`.
    #[derive(Default)]
    struct Recorder(RefCell<Vec<String>>, RefCell<Vec<serde_json::Value>>);

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
                "load-refs" => {
                    self.1.borrow_mut().push(value);
                    "refs".to_string()
                }
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
                    session.begin(None);
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
            // `main` checked out at the first chunk's newest commit, with no upstream, beside a
            // remote to fetch and push to.
            Ok(RepoRefs {
                locals: vec![LocalBranch {
                    name: "main".into(),
                    sha: "c9".into(),
                    short_sha: "c9".into(),
                    is_head: true,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    merged: true,
                }],
                default_remote: Some("origin".into()),
                ..RepoRefs::default()
            })
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
            session.begin(None),
            &recorder,
        );
        recorder.0.into_inner()
    }

    #[test]
    // Verifies: ADR-0016, ADR-0031, ADR-0037, G-13
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
        let older = session.begin(None);
        session.begin(None);
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

    #[test]
    // Verifies: ADR-0036, ADR-0035, ADR-0033
    fn the_refs_arrive_with_the_core_s_transfers_and_the_selection_with_its_operations() {
        let fake = Fake {
            chunks: vec![three(9)],
            ..Fake::default()
        };
        let session = Session::default();
        let recorder = Recorder::default();
        load(
            &fake,
            Path::new("/repo"),
            &session,
            session.begin(None),
            &recorder,
        );
        let payloads = recorder.1.into_inner();
        assert_eq!(payloads.len(), 1);
        let refs = fake.load_refs(Path::new("/repo")).unwrap();
        assert_eq!(payloads[0]["refs"], serde_json::to_value(&refs).unwrap());
        assert_eq!(
            payloads[0]["transfers"],
            serde_json::to_value(refs.transfers()).unwrap()
        );
        assert_eq!(
            payloads[0]["transfers"]["pull"]["reason"],
            "main has no upstream to pull from"
        );
        assert_eq!(payloads[0]["transfers"]["fetch"]["canStop"], true);
        let picked = session.select(1).unwrap();
        assert_eq!(picked.node.sha, "c8");
        assert_eq!(picked.operations, Some(refs.offered("c8")));
    }

    /// A sink that drops what it is told.
    struct Nowhere;

    impl Report for Nowhere {
        fn report<S: Serialize + Clone>(&self, _: &str, _: S) {}
    }

    #[test]
    // Verifies: ADR-0039
    fn a_guarded_host_checks_the_repository_again_before_every_command_that_reaches_it() {
        let refusing = Arc::new(AtomicBool::new(false));
        let refuses = Arc::clone(&refusing);
        let host = Host::guarded(Arc::new(move |_| {
            if refuses.load(Ordering::SeqCst) {
                Err("refused".into())
            } else {
                Ok(())
            }
        }));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _in = runtime.enter();
        // An empty folder: no repository, so a command let through fails in the git adapter.
        let dir = std::env::temp_dir().join(format!("ocelli-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            host.open_repo(dir.clone(), "d".into(), None, Nowhere),
            Ok(())
        );
        let let_through = runtime.block_on(host.get_file_diff("c".into(), "f".into()));
        assert!(let_through.is_err_and(|e| e != "refused"));

        refusing.store(true, Ordering::SeqCst);
        assert_eq!(
            runtime.block_on(host.get_file_diff("c".into(), "f".into())),
            Err("refused".into())
        );
        assert_eq!(
            runtime.block_on(host.select_commit("c".into(), &Nowhere)),
            Err("refused".into())
        );
        assert_eq!(
            host.open_repo(dir.clone(), "d".into(), None, Nowhere),
            Err("refused".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
