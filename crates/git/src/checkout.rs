//! Writing the index and the working directory through libgit2's checkout, which writes a link
//! as a link and a file with its mode, and takes a path as written, never as a pattern.

use crate::guard::nothing_in_progress;
use anyhow::{Result, bail};
use git2::build::CheckoutBuilder;
use git2::{CheckoutNotificationType, Commit, ErrorCode, Repository};
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
            .flat_map(|d| [d.old_file().path(), d.new_file().path()])
            .flatten()
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

    let mut in_the_way = Vec::new();
    let mut checkout = CheckoutBuilder::new();
    checkout
        .recreate_missing(true)
        .notify_on(CheckoutNotificationType::CONFLICT)
        .notify(|_, path, _, _, _| {
            if let Some(path) = path {
                in_the_way.push(path.display().to_string());
            }
            true
        });
    let checked_out = repo.checkout_tree(target.as_object(), Some(&mut checkout));
    drop(checkout);
    match checked_out {
        Err(e) if e.code() == ErrorCode::Conflict => {
            in_the_way.sort();
            bail!(
                "uncommitted changes to {} are in the way: commit or stash them first",
                in_the_way.join(", ")
            );
        }
        result => result?,
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
