//! Writing the index and the working directory through libgit2's checkout, which writes a link
//! as a link and a file with its mode, and takes a path as written, never as a pattern.

use crate::guard::nothing_in_progress;
use anyhow::{Result, bail};
use git2::build::CheckoutBuilder;
use git2::{Commit, ErrorCode, Repository};
use std::path::Path;

/// Checks `target`'s tree out over the checked-out one, the uncommitted changes carried along,
/// or refuses with nothing changed, naming what is in the way: what a switch and a fast-forward
/// both do to the working directory and the index.
pub(crate) fn check_out(repo: &Repository, target: &Commit) -> Result<()> {
    nothing_in_progress(repo, true)?;
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let goal = target.tree()?;
    let paths = |diff: git2::Diff| -> Vec<String> {
        diff.deltas()
            .flat_map(|d| [d.old_file(), d.new_file()])
            .filter_map(|f| crate::guard::diff_path(&f))
            .map(|p| p.display().to_string())
            .collect()
    };
    let changed = paths(repo.diff_tree_to_tree(head.as_ref(), Some(&goal), None)?);
    let mut staged: Vec<_> = paths(repo.diff_tree_to_index(head.as_ref(), None, None)?)
        .into_iter()
        .filter(|p| changed.contains(p))
        .collect();
    if !staged.is_empty() {
        staged.sort();
        staged.dedup();
        bail!(
            "staged changes to {} are in the way: commit or stash them first",
            staged.join(", ")
        );
    }

    if changed.is_empty() {
        return Ok(());
    }
    let written: Vec<String> = changed
        .iter()
        .filter(|p| {
            goal.get_path(Path::new(p.as_str()))
                .is_ok_and(|e| e.kind() != Some(git2::ObjectType::Tree))
        })
        .cloned()
        .collect();
    nothing_untracked_in_the_way(repo, &written)?;
    let mut checkout = CheckoutBuilder::new();
    // Only the paths the target changes: a file deleted in the working directory comes back
    // where the target changes it, and stays deleted where it does not, as `git switch` does.
    checkout.disable_pathspec_match(true);
    for path in &changed {
        #[expect(
            clippy::disallowed_methods,
            reason = "the pattern matching is turned off, and a path libgit2 read holds no NUL"
        )]
        checkout.path(path.as_str());
    }
    checkout.recreate_missing(true);
    let checked_out = repo.checkout_tree(target.as_object(), Some(&mut checkout));
    drop(checkout);
    match checked_out {
        Err(e) if e.code() == ErrorCode::Conflict => {
            let in_the_way = changed_in_the_working_directory(repo, &changed)?;
            if in_the_way.is_empty() {
                bail!("uncommitted changes are in the way: commit or stash them first");
            }
            bail!(
                "uncommitted changes to {} are in the way: commit or stash them first",
                in_the_way.join(", ")
            );
        }
        result => result?,
    }
    Ok(())
}

/// Which of `paths` the working directory changes or holds untracked, by name, sorted: what a
/// checkout libgit2 refused for a conflict found in its way. libgit2 would name them to a
/// notify callback, but git2 unwraps the path it hands that as UTF-8 on Windows, where a
/// release build aborts on the panic.
fn changed_in_the_working_directory(repo: &Repository, paths: &[String]) -> Result<Vec<String>> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(true);
    let mut found: Vec<String> = repo
        .statuses(Some(&mut options))?
        .iter()
        .filter(|e| e.status() != git2::Status::CURRENT && !e.status().is_ignored())
        .map(|e| String::from_utf8_lossy(e.path_bytes()).into_owned())
        .filter(|p| paths.contains(p))
        .collect();
    found.sort();
    found.dedup();
    Ok(found)
}

/// Refuses where a write would put a file at one of `files`, the paths its result holds a file at
/// and the checked-out commit does not hold so, and the working directory has a file the index
/// does not track in the way: a directory holding one where a file goes, or one where a directory
/// goes. libgit2 would stop halfway, or a cherry-pick commit without the file, where `git`
/// refuses for an untracked file and deletes an ignored one; Ocelli keeps both (ADR-0033).
pub(crate) fn nothing_untracked_in_the_way(repo: &Repository, files: &[String]) -> Result<()> {
    let Some(root) = repo.workdir() else {
        return Ok(());
    };
    let index = repo.index()?;
    let tracked = |p: &str| crate::git::index_entry(&index, p, 0).is_some();
    for path in files {
        let dir = root.join(path);
        if dir.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            let mut found = Vec::new();
            files_below(root, &dir, &mut found)?;
            let mut kept: Vec<_> = found.into_iter().filter(|f| !tracked(f)).collect();
            if !kept.is_empty() {
                kept.sort();
                bail!(
                    "{path} is a directory holding files the commit does not track, which a \
                     file would replace: move {} first",
                    kept.join(", ")
                );
            }
        }
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let above = parts[..i].join("/");
            let there = root.join(&above).symlink_metadata();
            if there.is_ok_and(|m| !m.is_dir()) && !tracked(&above) {
                bail!(
                    "{above} is a file the commit does not track where {path} needs a directory: \
                     move it first"
                );
            }
        }
    }
    Ok(())
}

/// The paths `index` holds a file at, at any stage, that `head` does not hold so: where a write
/// whose result is `index` puts a file.
pub(crate) fn files_written(head: &git2::Tree, index: &git2::Index) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in index.iter() {
        let path = String::from_utf8_lossy(&entry.path).into_owned();
        let same = head
            .get_path(Path::new(&path))
            .is_ok_and(|e| e.id() == entry.id && e.filemode() as u32 == entry.mode);
        if !same {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The files below `dir`, by their path from `root`, links among them, none followed.
fn files_below(root: &Path, dir: &Path, found: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.symlink_metadata()?.is_dir() {
            files_below(root, &path, found)?;
        } else if let Ok(name) = path.strip_prefix(root) {
            found.push(name.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Puts `path` in `index` as `tree` holds it, or takes it out where `tree` has no file there:
/// the path taken as written, never as a pattern, as `git reset -- <path>` takes it.
pub(crate) fn entry_as_in(index: &mut git2::Index, tree: &git2::Tree, path: &str) -> Result<()> {
    let at = Path::new(path);
    index.remove_path(at)?;
    if let Ok(entry) = tree.get_path(at)
        && matches!(
            entry.kind(),
            Some(git2::ObjectType::Blob | git2::ObjectType::Commit)
        )
    {
        let zero = git2::IndexTime::new(0, 0);
        index.add(&git2::IndexEntry {
            ctime: zero,
            mtime: zero,
            dev: 0,
            ino: 0,
            mode: entry.filemode() as u32,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: entry.id(),
            flags: 0,
            flags_extended: 0,
            path: path.as_bytes().to_vec(),
        })?;
    }
    Ok(())
}
