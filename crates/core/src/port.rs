//! The repository port (ADR-0009): what the core needs from a repository, owned by the core and
//! implemented by an adapter. Every path names a directory inside the repository to read.

use crate::model::{CommitData, DiffLine, FileChange, RepoRefs};
use anyhow::Result;
use std::path::Path;

/// Reads a repository's history and state.
pub trait RepositoryReader {
    /// Loads the commits to lay out, newest first, capped at [`crate::model::MAX_COMMITS`] and
    /// with the stash and working-directory nodes added (ADR-0024). `progress` receives one
    /// message per step of the load.
    fn load_commits(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
    ) -> Result<Vec<CommitData>>;

    /// Loads every ref category the repository sidebar shows.
    fn load_refs(&self, repo: &Path) -> Result<RepoRefs>;

    /// Loads the files a commit changed; [`crate::model::WORKDIR_SHA`] yields the uncommitted
    /// changes.
    fn load_commit_files(&self, repo: &Path, sha: &str) -> Result<Vec<FileChange>>;

    /// Loads the diff of one file of a commit; [`crate::model::WORKDIR_SHA`] yields the
    /// uncommitted diff.
    fn load_file_diff(&self, repo: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>>;
}
