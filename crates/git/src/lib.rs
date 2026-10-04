//! The repository port of `ocelli-core` implemented with git2 (ADR-0009, ADR-0019). git2 runs
//! no hooks, no `core.fsmonitor`, and no external diff or filter driver, so reading a repository
//! starts no program the repository names (Q-4).

pub mod git;
pub mod refs;

use anyhow::Result;
use ocelli_core::model::{CommitData, DiffLine, FileChange, RepoRefs};
use ocelli_core::port::RepositoryReader;
use std::path::Path;

/// The git2 implementation of [`RepositoryReader`].
#[derive(Debug, Default, Clone, Copy)]
pub struct GitReader;

impl RepositoryReader for GitReader {
    fn load_commits(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
    ) -> Result<Vec<CommitData>> {
        git::load_repository(repo, progress)
    }

    fn load_refs(&self, repo: &Path) -> Result<RepoRefs> {
        refs::load_repo_refs(repo)
    }

    fn load_commit_files(&self, repo: &Path, sha: &str) -> Result<Vec<FileChange>> {
        git::load_commit_files(repo, sha)
    }

    fn load_file_diff(&self, repo: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>> {
        git::load_file_diff(repo, sha, file_path)
    }
}
