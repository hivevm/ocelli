//! The repository port (ADR-0009): what the core needs from a repository, owned by the core and
//! implemented by an adapter. Every path names a directory inside the repository to read.

use crate::model::{CommitData, DiffLine, FileChange, RepoRefs};
use anyhow::Result;
use std::path::Path;

/// Reads a repository's history and state.
pub trait RepositoryReader {
    /// Loads the whole history in chunks of [`crate::model::CHUNK_COMMITS`] in walk order, the
    /// first with the stash and working-directory nodes added (ADR-0024), handing each to
    /// `on_chunk` as it is read; `on_chunk` returns whether to go on, so a new load session can cancel the walk
    /// (ADR-0016). `progress` receives one message per step.
    fn load_chunks(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
        on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
    ) -> Result<()>;

    /// The tip of the main line (ADR-0032): the commit of the local branch `origin/HEAD` names,
    /// else of `main`, else of `master`, else the checked-out commit; none where `HEAD` does not
    /// resolve either.
    fn load_main_tip(&self, repo: &Path) -> Result<Option<String>>;

    /// Loads every ref category the repository sidebar shows.
    fn load_refs(&self, repo: &Path) -> Result<RepoRefs>;

    /// Loads the files a commit changed; [`crate::model::WORKDIR_SHA`] yields the uncommitted
    /// changes.
    fn load_commit_files(&self, repo: &Path, sha: &str) -> Result<Vec<FileChange>>;

    /// Loads the diff of one file of a commit; [`crate::model::WORKDIR_SHA`] yields the
    /// uncommitted diff.
    fn load_file_diff(&self, repo: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>>;
}
