//! Writing the index and the working directory through libgit2's checkout, which writes a link
//! as a link and a file with its mode, and takes a path as written, never as a pattern.

use crate::guard::{bytes_to_path, diff_path, exact_path, nothing_in_progress};
use anyhow::{Result, bail};
use git2::build::CheckoutBuilder;
use git2::{Commit, ErrorCode, Repository};
use std::path::{Path, PathBuf};

/// Checks `target`'s tree out over the checked-out one, the uncommitted changes carried along,
/// or refuses with nothing changed, naming what is in the way: what a switch and a fast-forward
/// both do to the working directory and the index. Each path goes back to libgit2 in the bytes
/// it read, a name that is not UTF-8 among them, which Windows cannot hold and refuses.
pub(crate) fn check_out(repo: &Repository, target: &Commit) -> Result<()> {
    nothing_in_progress(repo, true)?;
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let goal = target.tree()?;
    let changed = repo
        .diff_tree_to_tree(head.as_ref(), Some(&goal), None)?
        .deltas()
        .flat_map(|d| [d.old_file(), d.new_file()])
        .filter_map(|f| f.path_bytes().map(exact_path))
        .collect::<Result<Vec<PathBuf>>>()?;
    let mut staged: Vec<String> = repo
        .diff_tree_to_index(head.as_ref(), None, None)?
        .deltas()
        .flat_map(|d| [d.old_file(), d.new_file()])
        .filter_map(|f| diff_path(&f))
        .filter(|p| changed.contains(p))
        .map(|p| named(&p))
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
    let written: Vec<PathBuf> = changed
        .iter()
        .filter(|p| {
            goal.get_path(p)
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
            reason = "the pattern matching is turned off, and a path libgit2 read holds no NUL \
                      and is UTF-8 on Windows, where git2 would unwrap it as such"
        )]
        checkout.path(path.as_path());
    }
    checkout.recreate_missing(true);
    let checked_out = crate::workdir::checkout_tree(repo, target, &mut checkout)?;
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

/// A path of the repository as a message names it: with forward slashes, lossily where it is not
/// UTF-8.
pub(crate) fn named(path: &Path) -> String {
    let name = path.to_string_lossy();
    if cfg!(windows) {
        name.replace('\\', "/")
    } else {
        name.into_owned()
    }
}

/// Which of `paths` the working directory changes or holds untracked, by name, sorted: what a
/// checkout libgit2 refused for a conflict found in its way. libgit2 would name them to a
/// notify callback, but git2 unwraps the path it hands that as UTF-8 on Windows, where a
/// release build aborts on the panic.
fn changed_in_the_working_directory(repo: &Repository, paths: &[PathBuf]) -> Result<Vec<String>> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(true);
    let mut found: Vec<String> = repo
        .statuses(Some(&mut options))?
        .iter()
        .filter(|e| e.status() != git2::Status::CURRENT && !e.status().is_ignored())
        .map(|e| bytes_to_path(e.path_bytes()))
        .filter(|p| paths.contains(p))
        .map(|p| named(&p))
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
pub(crate) fn nothing_untracked_in_the_way(repo: &Repository, files: &[PathBuf]) -> Result<()> {
    let Some(root) = repo.workdir() else {
        return Ok(());
    };
    let index = repo.index()?;
    let tracked = |p: &Path| crate::git::index_entry(&index, p, 0).is_some();
    let link = |p: &Path| {
        root.join(p)
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
    };
    for path in files {
        let mut above: Vec<&Path> = path
            .ancestors()
            .skip(1)
            .filter(|a| !a.as_os_str().is_empty())
            .collect();
        above.reverse();
        // A link in the way is the guard's of `workdir` to refuse, the write's last check before
        // libgit2; looking past one here would look outside the working directory.
        if above.iter().any(|a| link(a)) {
            continue;
        }
        let dir = root.join(path);
        if dir.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            let mut found = Vec::new();
            files_below(root, &dir, &mut found)?;
            let mut kept: Vec<String> = found
                .iter()
                .filter(|f| !tracked(f))
                .map(|f| named(f))
                .collect();
            if !kept.is_empty() {
                kept.sort();
                bail!(
                    "{} is a directory holding files the commit does not track, which a \
                     file would replace: move {} first",
                    named(path),
                    kept.join(", ")
                );
            }
        }
        for above in above {
            let there = root.join(above).symlink_metadata();
            if there.is_ok_and(|m| !m.is_dir()) && !tracked(above) {
                bail!(
                    "{} is a file the commit does not track where {} needs a directory: \
                     move it first",
                    named(above),
                    named(path)
                );
            }
        }
    }
    Ok(())
}

/// The paths `index` holds a file at, at any stage, that `head` does not hold so: where a write
/// whose result is `index` puts a file.
pub(crate) fn files_written(head: &git2::Tree, index: &git2::Index) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in index.iter() {
        let path = bytes_to_path(&entry.path);
        let same = head
            .get_path(&path)
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
fn files_below(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.symlink_metadata()?.is_dir() {
            files_below(root, &path, found)?;
        } else if let Ok(name) = path.strip_prefix(root) {
            found.push(name.to_path_buf());
        }
    }
    Ok(())
}

/// Puts `path` in `index` as `tree` holds it, or takes it out where `tree` has no file there:
/// the path taken as written, never as a pattern, as `git reset -- <path>` takes it.
pub(crate) fn entry_as_in(index: &mut git2::Index, tree: &git2::Tree, path: &Path) -> Result<()> {
    index.remove_path(path)?;
    if let Ok(entry) = tree.get_path(path)
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
            path: crate::guard::path_bytes(path),
        })?;
    }
    Ok(())
}
