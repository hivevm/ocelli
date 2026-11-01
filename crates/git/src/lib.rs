//! The repository ports of `ocelli-core` implemented with git2 (ADR-0008, ADR-0017, ADR-0031).
//! git2 runs no hooks, no `core.fsmonitor`, and no external diff or filter driver, so reading or
//! writing a repository starts no program the repository names (Q-4).

mod checkout;
pub mod confine;
mod conflicts;
pub mod git;
mod guard;
mod message;
mod operations;
pub mod refs;
mod remote;
mod stash;
mod wildmatch;
mod workdir;
mod write;

use anyhow::Result;
use ocelli_core::merge::MergeFile;
use ocelli_core::model::{CommitData, DiffLine, FileChange, RepoRefs};
use ocelli_core::port::{RepositoryReader, RepositoryWriter, Write};
use std::path::Path;

/// The git2 implementation of [`RepositoryReader`].
#[derive(Debug, Default, Clone, Copy)]
pub struct GitReader;

impl GitReader {
    /// The first chunk of the history alone (ADR-0016), for what measures or tests the newest
    /// commits; the front ends load through [`RepositoryReader::load_chunks`].
    pub fn load_first_chunk(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
    ) -> Result<Vec<CommitData>> {
        git::load_first_chunk(repo, progress)
    }
}

impl RepositoryReader for GitReader {
    fn load_chunks(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
        on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
    ) -> Result<()> {
        git::load_chunks(repo, progress, on_chunk)
    }

    fn load_main_tip(&self, repo: &Path) -> Result<Option<String>> {
        refs::load_main_tip(repo)
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

    fn load_merge_file(&self, repo: &Path, path: &str) -> Result<MergeFile> {
        conflicts::load_merge_file(repo, path)
    }
}

/// The git2 implementation of [`RepositoryWriter`] (ADR-0031).
#[derive(Debug, Default, Clone, Copy)]
pub struct GitWriter;

impl RepositoryWriter for GitWriter {
    fn write_reporting(
        &self,
        repo: &Path,
        write: &Write,
        progress: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Option<String>> {
        write::write(repo, write, progress)
    }
}
