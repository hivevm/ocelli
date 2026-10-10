//! The stash writes of the repository port, with git2: making a stash, and applying, popping,
//! and dropping one still where it was shown, refused where it would drop an operation in
//! progress, write past what it stashes, overwrite a staged change, or meet a file in its way
//! (ADR-0033).

use crate::guard::{NO_IDENTITY, nothing_in_progress};
use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    ErrorCode, ObjectType, Oid, Repository, StashApplyOptions, StashFlags, Status, StatusOptions,
    TreeWalkMode, TreeWalkResult,
};
use std::path::Path;

/// Stashes the uncommitted changes, the untracked files with them where `untracked` says so.
pub(crate) fn create(repo: &mut Repository, message: Option<&str>, untracked: bool) -> Result<()> {
    let stasher = repo.signature().context(NO_IDENTITY)?;
    let mut flags = StashFlags::DEFAULT;
    if untracked {
        flags |= StashFlags::INCLUDE_UNTRACKED;
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    // A stash during an operation in progress would drop the operation with it, as `git stash`
    // does, a bisect aside (ADR-0033).
    nothing_in_progress(repo, true)?;
    stash_keeps_everything(repo, untracked)?;
    match repo.stash_save2(&stasher, message, Some(flags)) {
        Err(e) if e.code() == ErrorCode::NotFound => bail!("there is nothing to stash"),
        result => result?,
    };
    // libgit2 leaves a staged change in the index where the working directory holds the file as
    // committed; the stash holds it now, and `git stash` resets to the checked-out commit.
    let head = repo.head()?.peel_to_commit()?;
    repo.reset(head.as_object(), git2::ResetType::Hard, None)?;
    Ok(())
}

/// Refuses a stash libgit2 would make by writing past what it stashes (ADR-0033): through a link
/// that stands where the index has a file or a directory, which writes the file's committed
/// content wherever the link points, out of the repository too; over an entry the index marks
/// skip-worktree, assume-unchanged, or intent-to-add, whose content it overwrites or misreads;
/// and, with the untracked files, over an ignored file in a directory only untracked files make,
/// which it deletes without stashing it.
fn stash_keeps_everything(repo: &Repository, untracked: bool) -> Result<()> {
    const ASSUME_UNCHANGED: u16 = 0x8000;
    const INTENT_TO_ADD: u16 = 1 << 13;
    const SKIP_WORKTREE: u16 = 1 << 14;
    const LINK: u32 = 0o120000;
    let root = repo
        .workdir()
        .context("a bare repository has no working directory to stash")?;
    let index = repo.index()?;
    let mut tracked_dirs = std::collections::HashSet::new();
    for entry in index.iter() {
        let path = String::from_utf8_lossy(&entry.path).into_owned();
        let marked = if entry.flags_extended & SKIP_WORKTREE != 0 {
            Some("skip-worktree, or outside a sparse checkout")
        } else if entry.flags_extended & INTENT_TO_ADD != 0 {
            Some("intent-to-add")
        } else if entry.flags & ASSUME_UNCHANGED != 0 {
            Some("assume-unchanged")
        } else {
            None
        };
        if let Some(mark) = marked {
            bail!("{path} is marked {mark} in the index, which a stash would lose: stash with git");
        }
        let mut dir = root.to_path_buf();
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            dir.push(part);
            let last = i + 1 == parts.len();
            let is_link = dir
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink());
            if is_link && (!last || entry.mode != LINK) {
                bail!(
                    "{} is a link where the index has {}: a stash would write through it, \
                     so move it first",
                    dir.strip_prefix(root).unwrap_or(&dir).display(),
                    if last { "a file" } else { "a directory" }
                );
            }
            if !last {
                tracked_dirs.insert(parts[..=i].join("/"));
            }
        }
    }
    if !untracked {
        return Ok(());
    }
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .recurse_ignored_dirs(true);
    let statuses = repo.statuses(Some(&mut options))?;
    let of = |flag: Status| -> Vec<String> {
        statuses
            .iter()
            .filter(|e| e.status().contains(flag))
            .filter_map(|e| e.path().map(str::to_string))
            .collect()
    };
    let (new, ignored) = (of(Status::WT_NEW), of(Status::IGNORED));
    for file in &ignored {
        let parts: Vec<&str> = file.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir = parts[..=i].join("/");
            let prefix = format!("{dir}/");
            if !tracked_dirs.contains(&dir) && new.iter().any(|n| n.starts_with(&prefix)) {
                bail!(
                    "{file} is ignored in {dir}, which holds only untracked files: a stash of \
                     the untracked files would delete it, so move it first"
                );
            }
        }
    }
    Ok(())
}

/// Refuses unless the stash at `index` is still the entry the developer saw, by its commit.
fn stash_as_shown(repo: &Repository, index: usize, commit: &str) -> Result<()> {
    let found = crate::guard::stashes(repo)?
        .get(index)
        .map(|(_, oid)| oid.to_string());
    if found.as_deref() != Some(commit) {
        bail!(
            "stash@{{{index}}} is no longer the stash shown: reload to see the stashes as they are"
        );
    }
    Ok(())
}

/// Refuses a stash that would not apply cleanly, before anything changes: libgit2 writes the
/// untracked files first, then the changes, and leaves a conflict in the working directory, so
/// whatever would stop it halfway or overwrite something is looked for here. The index holds
/// nothing staged, the stash's changes merge with the checked-out commit without a conflict, and
/// every path the stash writes is free on disk: a file there is one the checked-out commit tracks
/// unchanged, by exactly that name, and no file stands where it needs a directory nor a
/// directory where it writes a file. Untracked and ignored files in its way refuse it too, as
/// does a file whose name differs only in case on a file system that does not tell the two
/// apart.
fn check_applies(repo: &Repository, index: usize, commit: &str) -> Result<()> {
    let stash = repo.find_commit(Oid::from_str(commit)?)?;
    let base = stash.parent(0)?.tree()?;
    let changes = stash.tree()?;
    let head = repo.head()?.peel_to_tree()?;
    let root = repo
        .workdir()
        .context("a bare repository has no working directory to apply a stash to")?;
    let staged = repo.diff_tree_to_index(Some(&head), None, None)?;
    if staged.deltas().len() > 0 {
        bail!("the index holds staged changes: commit or unstage them before applying a stash");
    }
    let name = format!("stash@{{{index}}}");
    if repo
        .merge_trees(&base, &head, &changes, None)?
        .has_conflicts()
    {
        bail!("{name} conflicts with the checked-out commit, so it is not applied");
    }

    let mut written = Vec::new();
    for delta in repo
        .diff_tree_to_tree(Some(&base), Some(&changes), None)?
        .deltas()
    {
        for file in [delta.old_file(), delta.new_file()] {
            if let Some(path) = crate::guard::diff_path(&file) {
                written.push(path);
            }
        }
    }
    // The third parent, where there is one, holds the untracked files the stash took along;
    // one the checked-out commit tracks by now would be overwritten.
    if let Ok(untracked) = stash.parent(2) {
        let mut files = Vec::new();
        untracked
            .tree()?
            .walk(TreeWalkMode::PreOrder, |dir, entry| {
                if entry.kind() == Some(ObjectType::Blob)
                    && let Some(name) = entry.name()
                {
                    files.push(Path::new(dir).join(name));
                }
                TreeWalkResult::Ok
            })?;
        if let Some(tracked) = files.iter().find(|f| head.get_path(f).is_ok()) {
            bail!(
                "{} is tracked by now, so {name} would overwrite it",
                tracked.display()
            );
        }
        written.extend(files);
    }

    for path in &written {
        for dir in path
            .ancestors()
            .skip(1)
            .filter(|d| !d.as_os_str().is_empty())
        {
            if root.join(dir).symlink_metadata().is_ok_and(|m| !m.is_dir()) {
                bail!("{} stands where {name} needs a directory", dir.display());
            }
        }
        let tracked = head.get_path(path).is_ok();
        match root.join(path).symlink_metadata() {
            Ok(meta) if meta.is_dir() => {
                bail!(
                    "{} is a directory where {name} writes a file",
                    path.display()
                )
            }
            Ok(_) if !tracked => {
                bail!(
                    "{} is not tracked, so {name} would overwrite it",
                    path.display()
                )
            }
            Ok(_) if repo.status_file(path).ok() != Some(Status::CURRENT) => {
                bail!(
                    "{} has changes of its own that {name} would overwrite",
                    path.display()
                )
            }
            Err(_) if tracked => {
                bail!(
                    "{} has changes of its own that {name} would overwrite",
                    path.display()
                )
            }
            _ => {}
        }
    }
    Ok(())
}

/// Applies the stash at `index`, still the entry the developer saw by `commit`, and drops it
/// after where `drop` asks, as `git stash apply` and `git stash pop` do, or refuses with nothing
/// changed (ADR-0033).
pub(crate) fn apply(repo: &mut Repository, index: usize, commit: &str, drop: bool) -> Result<()> {
    stash_as_shown(repo, index, commit)?;
    check_applies(repo, index, commit)?;
    // A second guard behind the check: an ignored file is never overwritten.
    let mut checkout = CheckoutBuilder::new();
    checkout.overwrite_ignored(false);
    let mut options = StashApplyOptions::new();
    options.checkout_options(checkout);
    repo.stash_apply(index, Some(&mut options))?;
    if drop {
        // Applied by now: a stash pushed meanwhile moves the entry, which then stays.
        stash_as_shown(repo, index, commit).context("the stash is applied but not dropped")?;
        repo.stash_drop(index)?;
    }
    Ok(())
}

/// Drops the stash at `index`, still the entry the developer saw by `commit`.
pub(crate) fn drop(repo: &mut Repository, index: usize, commit: &str) -> Result<()> {
    stash_as_shown(repo, index, commit)?;
    repo.stash_drop(index)?;
    Ok(())
}
