//! The writes of the repository port, with git2 (ADR-0033). libgit2 runs no hook and no program
//! a repository's configuration names, so writing, like reading, runs no code the repository
//! brings (Q-4). Every write checks the state it expects before it changes anything.

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    ErrorCode, ObjectType, Oid, Reference, Repository, StashApplyOptions, StashFlags, Status,
    TreeWalkMode, TreeWalkResult,
};
use ocelli_core::port::Write;
use std::path::Path;

/// Performs `write` on the repository at `repo_path`, or refuses it with nothing changed.
pub fn write(repo_path: &Path, write: &Write) -> Result<()> {
    let mut repo = Repository::discover(repo_path)?;
    match write {
        Write::CreateTag {
            name,
            commit,
            message,
        } => create_tag(&repo, name, commit, message.as_deref()),
        Write::DeleteTag {
            name,
            commit,
            annotated,
        } => delete_tag(&repo, name, commit, *annotated),
        Write::CreateStash { message, untracked } => {
            create_stash(&mut repo, message.as_deref(), *untracked)
        }
        Write::ApplyStash {
            index,
            commit,
            drop,
        } => {
            stash_as_shown(&mut repo, *index, commit)?;
            check_applies(&repo, *index, commit)?;
            // A second guard behind the check: an ignored file is never overwritten.
            let mut checkout = CheckoutBuilder::new();
            checkout.overwrite_ignored(false);
            let mut options = StashApplyOptions::new();
            options.checkout_options(checkout);
            repo.stash_apply(*index, Some(&mut options))?;
            if *drop {
                // Applied by now: a stash pushed meanwhile moves the entry, which then stays.
                stash_as_shown(&mut repo, *index, commit)
                    .context("the stash is applied but not dropped")?;
                repo.stash_drop(*index)?;
            }
            Ok(())
        }
        Write::DropStash { index, commit } => {
            stash_as_shown(&mut repo, *index, commit)?;
            repo.stash_drop(*index)?;
            Ok(())
        }
    }
}

const NO_IDENTITY: &str = "no identity is configured to write with: set user.name and user.email";

fn create_tag(repo: &Repository, name: &str, commit: &str, message: Option<&str>) -> Result<()> {
    let refname = format!("refs/tags/{name}");
    // `git tag` refuses a leading dash too, which would read as an option.
    if name.starts_with('-') || !Reference::is_valid_name(&refname) {
        bail!("'{name}' is not a name a tag can have");
    }
    if repo.find_reference(&refname).is_ok() {
        bail!("a tag named '{name}' exists already");
    }
    let target = Oid::from_str(commit)
        .ok()
        .and_then(|oid| repo.find_object(oid, Some(ObjectType::Commit)).ok())
        .with_context(|| format!("the commit {commit} is no longer in the repository"))?;
    // Cleaned up as `git tag` cleans it: comment lines, surrounding blank lines, and trailing
    // spaces go, and it ends in a newline.
    let comment = comment_string(repo)?;
    let message = message
        .map(|m| {
            let kept: Vec<_> = m.lines().filter(|l| !l.starts_with(&comment)).collect();
            git2::message_prettify(kept.join("\n"), None)
        })
        .transpose()?;
    match message.filter(|m| !m.is_empty()) {
        Some(message) => {
            let tagger = repo.signature().context(NO_IDENTITY)?;
            repo.tag(name, &target, &tagger, &message, false)?;
        }
        None => {
            repo.tag_lightweight(name, &target, false)?;
        }
    }
    Ok(())
}

/// What starts a comment line in a message, as `git` reads it: `core.commentString` or
/// `core.commentChar`, whichever the configuration sets last, and `#` where neither is set or
/// it is `auto`, which `git tag` does not adjust.
fn comment_string(repo: &Repository) -> Result<String> {
    let config = repo.config()?.snapshot()?;
    let mut comment = None;
    config
        .entries(Some("core\\.comment(char|string)"))?
        .for_each(|entry| comment = entry.value().map(str::to_string))?;
    Ok(comment
        .filter(|c| !c.is_empty() && c != "auto")
        .unwrap_or_else(|| "#".to_string()))
}

/// Deletes the tag, if it is still the one the developer saw: on `commit`, and annotated or not
/// as `annotated` says. libgit2 deletes the reference only while it still holds the value read
/// here, so a tag moved in between is not deleted either; it drops the tag's reflog before that
/// comparison, which only a repository logging every ref update keeps for tags.
fn delete_tag(repo: &Repository, name: &str, commit: &str, annotated: bool) -> Result<()> {
    let mut reference = repo
        .find_reference(&format!("refs/tags/{name}"))
        .with_context(|| format!("there is no tag named '{name}' any more"))?;
    let points_at = reference
        .peel(ObjectType::Commit)
        .map(|c| c.id().to_string());
    let is_annotated = reference.peel(ObjectType::Tag).is_ok();
    if points_at.ok().as_deref() != Some(commit) || is_annotated != annotated {
        bail!("the tag '{name}' is no longer the one shown: reload to see it as it is");
    }
    reference.delete()?;
    Ok(())
}

/// Stashes the uncommitted changes, the untracked files with them where `untracked` says so.
fn create_stash(repo: &mut Repository, message: Option<&str>, untracked: bool) -> Result<()> {
    let stasher = repo.signature().context(NO_IDENTITY)?;
    let mut flags = StashFlags::DEFAULT;
    if untracked {
        flags |= StashFlags::INCLUDE_UNTRACKED;
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    match repo.stash_save2(&stasher, message, Some(flags)) {
        Err(e) if e.code() == ErrorCode::NotFound => bail!("there is nothing to stash"),
        result => result.map(|_| ()).map_err(Into::into),
    }
}

/// Refuses unless the stash at `index` is still the entry the developer saw, by its commit.
fn stash_as_shown(repo: &mut Repository, index: usize, commit: &str) -> Result<()> {
    let mut found = None;
    repo.stash_foreach(|i, _, oid| {
        if i == index {
            found = Some(oid.to_string());
        }
        true
    })?;
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
            if let Some(path) = file.path() {
                written.push(path.to_path_buf());
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
