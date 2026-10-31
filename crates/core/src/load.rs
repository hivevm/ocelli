//! A load session (ADR-0016): the history of a repository read chunk by chunk through the
//! repository port, each chunk laid out as it arrives (ADR-0015) and taken into a [`History`]
//! (ADR-0031). [`load`] runs one on the thread that reads; [`LoadSession`] is what a front end
//! keeps of it. Both front ends and the measurement load this way, so what a load does, and what
//! its status line says, exists once.

use crate::graph::{LaidChunk, Layout};
use crate::history::History;
use crate::model::{CommitData, RepoRefs};
use crate::port::RepositoryReader;
use std::cell::{Cell, RefCell};
use std::path::Path;

/// What a load hands over as it goes.
#[derive(Debug)]
pub enum Event {
    /// A step of the walk before the first chunk is laid out.
    Progress(String),
    /// A chunk laid out below the ones before.
    Chunk(LaidChunk),
    /// The refs, read right after the first chunk is handed over, so that the two describe the
    /// same state while the first chunk shows without waiting for them.
    Refs(Box<RepoRefs>),
    /// The load ended: the whole history read, or why it stopped.
    Ended(Result<(), String>),
}

/// Loads the history of `path` through `reader`, lays each chunk out, and hands every event to
/// `on`; what it returns after a chunk or the refs says whether to go on, and the load stops when
/// it is false, as when another repository is opened. Refs that cannot be read stop the load after
/// the first chunk. The layout is gone before the end is handed over, so what it held is free by
/// then.
pub fn load(reader: &impl RepositoryReader, path: &Path, on: &mut dyn FnMut(Event) -> bool) {
    // The walk calls the chunk and the progress callbacks one at a time, never one inside the
    // other, so they share `on` and what they know of the load through cells.
    // The main line's tip is named before the walk, so the first chunk is laid out with it
    // (ADR-0032).
    let tip = match reader.load_main_tip(path) {
        Ok(tip) => tip,
        Err(e) => {
            on(Event::Ended(Err(e.to_string())));
            return;
        }
    };
    let on = RefCell::new(on);
    let first = Cell::new(true);
    let stopped = RefCell::new(None::<String>);
    let result = {
        let mut layout = tip.map_or_else(Layout::new, Layout::with_main_line);
        let mut on_chunk = |commits: Vec<CommitData>| {
            let go_on = on.borrow_mut()(Event::Chunk(layout.lay_out(commits)));
            if !first.replace(false) || !go_on {
                return go_on;
            }
            match reader.load_refs(path) {
                Ok(refs) => on.borrow_mut()(Event::Refs(Box::new(refs))),
                Err(e) => {
                    *stopped.borrow_mut() = Some(e.to_string());
                    false
                }
            }
        };
        // Until the first chunk shows, the walk's progress is the status; after it, the chunks
        // are.
        let mut progress = |msg: String| {
            if first.get() {
                on.borrow_mut()(Event::Progress(msg));
            }
        };
        reader.load_chunks(path, &mut progress, &mut on_chunk)
    };
    let result = result
        .map_err(|e| e.to_string())
        .and_then(|()| stopped.take().map_or(Ok(()), Err));
    on.borrow_mut()(Event::Ended(result));
}

/// What a front end keeps of a load session: the history laid out so far and how far the load
/// has come.
#[derive(Debug, Default)]
pub struct LoadSession {
    history: History,
    chunks: usize,
    ended: Option<Result<(), String>>,
    /// The commit to select again once a chunk lays it out, after a write (ADR-0033).
    select_again: Option<String>,
}

impl LoadSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in the next chunk and returns the rows laid out before it, from which a search or a
    /// lineage grows.
    pub fn append(&mut self, chunk: LaidChunk) -> usize {
        let from = self.history.rows();
        self.history.append(chunk);
        self.chunks += 1;
        from
    }

    /// Ends the session, the whole history read or the reason it stopped. A commit to select
    /// again that no chunk laid out is given up.
    pub fn end(&mut self, result: Result<(), String>) {
        self.ended = Some(result);
        self.select_again = None;
    }

    /// Asks for `sha` to be selected again once a chunk lays it out: the commit selected when a
    /// write was done, which the load after it shows again (ADR-0033).
    pub fn select_again(&mut self, sha: String) {
        self.select_again = Some(sha);
    }

    /// Gives the commit to select again up, as when the developer selects another.
    pub fn keep_selection(&mut self) {
        self.select_again = None;
    }

    /// The row of the commit to select again, once a chunk has laid it out; answered once.
    pub fn row_to_select(&mut self) -> Option<usize> {
        let row = self.history.row_of(self.select_again.as_deref()?)?;
        self.select_again = None;
        Some(row)
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Whether further chunks may still come.
    pub fn loading(&self) -> bool {
        self.ended.is_none()
    }

    /// Whether the load read the whole history.
    pub fn complete(&self) -> bool {
        matches!(self.ended, Some(Ok(())))
    }

    /// The status line: the number of commits laid out, while loading the chunks they came in,
    /// and why the load stopped where it did. It starts with the number of commits and
    /// ` commits`, the shape `scripts/measure-memory.sh` reads.
    pub fn status(&self) -> String {
        let rows = self.history.rows();
        match &self.ended {
            None => {
                let unit = if self.chunks == 1 { "chunk" } else { "chunks" };
                format!(
                    "{rows} commits in {} {unit} — loading older history…",
                    self.chunks
                )
            }
            Some(Ok(())) => format!("{rows} commits"),
            Some(Err(e)) => format!("{rows} commits — loading stopped: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::commit;
    use crate::model::{DiffLine, FileChange};
    use anyhow::{Result, bail};

    /// A repository whose walk yields `chunks`, then fails if `walk_fails`, and whose refs fail
    /// if `refs_fail`; it counts the chunks the load took.
    #[derive(Default)]
    struct Fake {
        chunks: Vec<Vec<CommitData>>,
        walk_fails: bool,
        refs_fail: bool,
        taken: Cell<usize>,
    }

    impl RepositoryReader for Fake {
        fn load_chunks(
            &self,
            _: &Path,
            progress: &mut dyn FnMut(String),
            on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
        ) -> Result<()> {
            progress("walking".into());
            for chunk in &self.chunks {
                self.taken.set(self.taken.get() + 1);
                if !on_chunk(chunk.clone()) {
                    return Ok(());
                }
                progress("walking on".into());
            }
            if self.walk_fails {
                bail!("walk failed");
            }
            Ok(())
        }

        fn load_main_tip(&self, _: &Path) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        fn load_refs(&self, _: &Path) -> Result<RepoRefs> {
            if self.refs_fail {
                bail!("refs failed");
            }
            Ok(RepoRefs::default())
        }

        fn load_commit_files(&self, _: &Path, _: &str) -> Result<Vec<FileChange>> {
            unimplemented!()
        }

        fn load_file_diff(&self, _: &Path, _: &str, _: &str) -> Result<Vec<DiffLine>> {
            unimplemented!()
        }
    }

    /// Three commits `c{top}` down to `c{top - 2}`, each on the next.
    fn three(top: usize) -> Vec<CommitData> {
        (top - 2..=top)
            .rev()
            .map(|i| commit(&format!("c{i}"), i as i64, &[&format!("c{}", i - 1)]))
            .collect()
    }

    /// What the load hands over, one word per event, and a session fed with it.
    fn run(fake: &Fake, go_on: usize) -> (Vec<String>, LoadSession) {
        let mut seen = Vec::new();
        let mut session = LoadSession::new();
        load(fake, Path::new("/repo"), &mut |event| {
            seen.push(match event {
                Event::Progress(p) => format!("progress {p}"),
                Event::Chunk(chunk) => {
                    session.append(chunk);
                    format!("chunk {}", session.history().rows())
                }
                Event::Refs(_) => "refs".into(),
                Event::Ended(result) => {
                    session.end(result);
                    "ended".into()
                }
            });
            seen.len() < go_on
        });
        (seen, session)
    }

    #[test]
    // Verifies: ADR-0016, ADR-0015, G-17
    fn a_load_hands_over_each_chunk_laid_out_the_refs_with_the_first_and_its_end() {
        let fake = Fake {
            chunks: vec![three(9), three(6)],
            ..Fake::default()
        };
        let (seen, session) = run(&fake, usize::MAX);
        assert_eq!(
            seen,
            ["progress walking", "chunk 3", "refs", "chunk 6", "ended"]
        );
        assert_eq!(session.status(), "6 commits");
        assert!(session.complete() && !session.loading());
    }

    #[test]
    // Verifies: ADR-0016, G-17
    fn the_status_counts_the_chunks_while_loading() {
        let mut session = LoadSession::new();
        session.append(Layout::new().lay_out(three(9)));
        assert_eq!(
            session.status(),
            "3 commits in 1 chunk — loading older history…"
        );
        assert!(session.loading());
    }

    #[test]
    // Verifies: ADR-0016, G-17
    fn a_walk_failing_after_the_first_chunk_keeps_it_and_says_why() {
        let fake = Fake {
            chunks: vec![three(9)],
            walk_fails: true,
            ..Fake::default()
        };
        let (_, session) = run(&fake, usize::MAX);
        assert_eq!(session.status(), "3 commits — loading stopped: walk failed");
        assert!(!session.complete());
    }

    #[test]
    // Verifies: ADR-0016
    fn refs_failing_stop_the_load_after_the_first_chunk_and_say_why() {
        let fake = Fake {
            chunks: vec![three(9), three(6)],
            refs_fail: true,
            ..Fake::default()
        };
        let (seen, session) = run(&fake, usize::MAX);
        assert_eq!(seen, ["progress walking", "chunk 3", "ended"]);
        assert_eq!(session.status(), "3 commits — loading stopped: refs failed");
        assert_eq!(fake.taken.get(), 1);
    }

    #[test]
    // Verifies: ADR-0016
    fn a_walk_failing_before_the_first_chunk_ends_with_the_error() {
        let fake = Fake {
            walk_fails: true,
            ..Fake::default()
        };
        let mut ended = None;
        load(&fake, Path::new("/repo"), &mut |event| {
            if let Event::Ended(result) = event {
                ended = Some(result);
            }
            true
        });
        assert_eq!(ended, Some(Err("walk failed".to_string())));
    }

    #[test]
    // Verifies: ADR-0016
    fn the_load_stops_when_told_to() {
        let fake = Fake {
            chunks: vec![three(9), three(6), three(3)],
            ..Fake::default()
        };
        run(&fake, 3);
        assert_eq!(fake.taken.get(), 1, "the walk stopped after the refs");
    }

    /// A session with `select_again` asked for, fed `chunks` one after another, the developer
    /// choosing a commit before chunk `keep_after`; the rows it names after each.
    fn rows_named(
        select_again: &str,
        chunks: &[usize],
        keep_after: Option<usize>,
    ) -> Vec<Option<usize>> {
        let mut session = LoadSession::new();
        let mut layout = Layout::new();
        session.select_again(select_again.to_string());
        let mut named = Vec::new();
        for (i, &top) in chunks.iter().enumerate() {
            if keep_after == Some(i) {
                session.keep_selection();
            }
            session.append(layout.lay_out(three(top)));
            named.push(session.row_to_select());
        }
        named
    }

    #[test]
    // Verifies: ADR-draft-merge-editor
    fn the_commit_to_select_again_is_named_once_its_chunk_lays_it_out() {
        assert_eq!(rows_named("c4", &[9, 6, 3], None), [None, Some(5), None]);
    }

    #[test]
    // Verifies: ADR-draft-merge-editor
    fn the_developers_own_choice_or_the_end_gives_the_commit_up() {
        assert_eq!(rows_named("c4", &[9, 6], Some(1)), [None, None]);
        let mut session = LoadSession::new();
        session.select_again("c1".to_string());
        session.append(Layout::new().lay_out(three(9)));
        session.end(Ok(()));
        assert_eq!(session.row_to_select(), None);
    }
}
