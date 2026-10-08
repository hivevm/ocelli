//! The writes of the repository port, with git2 (ADR-0033). libgit2 runs no hook and no program
//! a repository's configuration names, so writing, like reading, runs no code the repository
//! brings (Q-4). Every write checks the state it expects before it changes anything.

use crate::checkout::{check_out, entry_as_in};
use crate::git::{INDEX, WORKTREE};
use crate::guard::{
    NO_IDENTITY, as_shown, at_tip, branch_as_shown, not_checked_out_elsewhere, nothing_in_progress,
    refname_of, refuse_where_signing_is_asked, valid_branch_name,
};
use crate::message::{Made, cleaned, stripped};
use anyhow::{Context, Result, bail};
use git2::{BranchType, Commit, ObjectType, Oid, Reference, Repository, Status};
use ocelli_core::port::Write;
use std::path::Path;

/// Performs `write` on the repository at `repo_path`, or refuses it with nothing changed; returns
/// the commit it made, where it made one.
pub fn write(
    repo_path: &Path,
    write: &Write,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<String>> {
    let mut repo = crate::confine::open(repo_path)?;
    let done = match write {
        Write::Fetch { remote } => crate::remote::fetch(&repo, remote, progress),
        Write::Pull { branch, tip } => crate::remote::pull(&repo, branch, tip, progress),
        Write::Push { branch, tip } => crate::remote::push(&repo, branch, tip, progress),
        Write::Merge {
            branch,
            tip,
            commit,
            name,
        } => return crate::operations::merge(&repo, branch, tip, commit, name.as_deref()),
        Write::Rebase { branch, tip, onto } => {
            return crate::operations::rebase(&repo, branch, tip, onto);
        }
        Write::CherryPick {
            branch,
            tip,
            commit,
        } => return crate::operations::cherry_pick(&repo, branch, tip, commit),
        Write::Revert {
            branch,
            tip,
            commit,
        } => return crate::operations::revert(&repo, branch, tip, commit),
        Write::Reset {
            branch,
            tip,
            commit,
            mode,
        } => return crate::operations::reset(&repo, branch, tip, commit, *mode),
        Write::Resolve {
            path,
            conflict,
            take,
        } => crate::conflicts::resolve(&repo, path, conflict, *take),
        Write::Continue { operation, head } => {
            return crate::conflicts::continue_stopped(&repo, *operation, head);
        }
        Write::Abort { operation, head } => crate::conflicts::abort(&repo, *operation, head),
        Write::Commit { message, head } => {
            return commit(&repo, message, head.as_deref()).map(Some);
        }
        Write::CreateBranch {
            name,
            commit,
            switch,
        } => create_branch(&repo, name, commit, *switch),
        Write::SwitchBranch { name, tip } => {
            let branch = branch_as_shown(&repo, name, tip)?;
            let target = repo.find_commit(Oid::from_str(tip)?)?;
            switch_to(&repo, &target, Some(&refname_of(&branch)?))
        }
        Write::Detach { commit } => {
            let target = Oid::from_str(commit)
                .ok()
                .and_then(|oid| repo.find_commit(oid).ok())
                .with_context(|| format!("the commit {commit} is no longer in the repository"))?;
            switch_to(&repo, &target, None)
        }
        Write::TrackRemote {
            remote,
            branch,
            tip,
        } => track_remote(&repo, remote, branch, tip),
        Write::DeleteBranch { name, tip, merged } => delete_branch(&repo, name, tip, *merged),
        Write::StageFile { path, shown } => {
            as_shown(&repo, path, shown, |s| s.workdir)?;
            stage_file(&repo, path)
        }
        Write::UnstageFile { path, shown } => {
            as_shown(&repo, path, shown, |s| s.index)?;
            unstage_file(&repo, path)
        }
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
            crate::stash::create(&mut repo, message.as_deref(), *untracked)
        }
        Write::ApplyStash {
            index,
            commit,
            drop,
        } => crate::stash::apply(&mut repo, *index, commit, *drop),
        Write::DropStash { index, commit } => crate::stash::drop(&mut repo, *index, commit),
    };
    done.map(|()| None)
}

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
    let message = message.map(|m| stripped(repo, m)).transpose()?;
    match message.filter(|m| !m.is_empty()) {
        Some(message) => {
            refuse_where_signing_is_asked(repo, "tag.gpgSign", "an annotated tag")?;
            refuse_where_signing_is_asked(repo, "tag.forceSignAnnotated", "an annotated tag")?;
            let tagger = repo.signature().context(NO_IDENTITY)?;
            repo.tag(name, &target, &tagger, &message, false)?;
        }
        None => {
            repo.tag_lightweight(name, &target, false)?;
        }
    }
    Ok(())
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

/// Stages the whole file at `path`, as `git add` does, removing it from the index where the
/// working directory deleted it; refused where it holds no change the index does not.
fn stage_file(repo: &Repository, path: &str) -> Result<()> {
    let status = repo.status_file(Path::new(path)).unwrap_or(Status::empty());
    if !status.intersects(WORKTREE) {
        bail!("{path} has no change to stage");
    }
    let mut index = repo.index()?;
    if status.contains(Status::WT_DELETED) {
        index.remove_path(Path::new(path))?;
    } else {
        index.add_path(Path::new(path))?;
    }
    index.write()?;
    Ok(())
}

/// Takes the file at `path` out of the index, back to the checked-out commit's, as
/// `git restore --staged` does; refused where the index holds no change of it.
fn unstage_file(repo: &Repository, path: &str) -> Result<()> {
    let status = repo.status_file(Path::new(path)).unwrap_or(Status::empty());
    if !status.intersects(INDEX) {
        bail!("{path} is not staged");
    }
    match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
        Some(head) => {
            let mut index = repo.index()?;
            entry_as_in(&mut index, &head.tree()?, path)?;
            index.write()?;
        }
        None => {
            let mut index = repo.index()?;
            index.remove_path(Path::new(path))?;
            index.write()?;
        }
    }
    Ok(())
}

/// Commits the index with `message` by the configured identity, where the checked-out commit is
/// still `head`, and returns the commit made. libgit2 moves the branch only while it still points
/// at `head`, so a commit made elsewhere meanwhile is not lost either.
fn commit(repo: &Repository, message: &str, head: Option<&str>) -> Result<String> {
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if parent.as_ref().map(|c| c.id().to_string()).as_deref() != head {
        bail!("the checked-out commit is no longer the one shown: reload to see it as it is");
    }
    // A bisect leaves commits alone; every other operation in progress records its own.
    nothing_in_progress(repo, true)?;
    refuse_where_signing_is_asked(repo, "commit.gpgSign", "a commit")?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        bail!("the index holds conflicts: resolve them before committing");
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree.id())
        || parent.is_none() && tree.is_empty()
    {
        bail!("nothing is staged to commit");
    }
    let message = cleaned(repo, message, Made::Given)?;
    if message.is_empty() {
        bail!("a commit needs a message");
    }
    let author = repo.signature().context(NO_IDENTITY)?;
    let parents: Vec<_> = parent.iter().collect();
    let made = repo.commit(Some("HEAD"), &author, &author, &message, &tree, &parents)?;
    Ok(made.to_string())
}

/// Checks `target` out and points `HEAD` at the branch `refname`, or at `target` itself where
/// there is none, as `git switch` does: the uncommitted changes go along where the target touches
/// none of the paths they touch, and a file deleted in the working directory comes back where the
/// target changes it. What would stop it is looked for first, so a switch refused changes
/// nothing: a staged change to a path the target changes, which libgit2 would overwrite, and the
/// branch checked out in another working tree; libgit2 then finds every other path in the way
/// before it writes anything, and the refusal names the paths.
fn switch_to(repo: &Repository, target: &Commit, refname: Option<&str>) -> Result<()> {
    if let Some(refname) = refname {
        not_checked_out_elsewhere(repo, refname)?;
    }
    check_out(repo, target)?;
    match refname {
        Some(refname) => repo.set_head(refname)?,
        None => repo.set_head_detached(target.id())?,
    }
    Ok(())
}

/// Switches to the branch `name` just created, deleting it again where the switch is refused,
/// so that a refusal leaves nothing behind.
fn switch_to_new(repo: &Repository, name: &str, target: &Commit) -> Result<()> {
    let created = repo.find_branch(name, BranchType::Local)?;
    let refname = refname_of(&created)?;
    switch_to(repo, target, Some(&refname)).inspect_err(|_| {
        if let Ok(mut created) = repo.find_branch(name, BranchType::Local) {
            let _ = created.delete();
        }
    })
}

/// Creates the branch `name` at `commit`, and switches to it where `switch` says so. Created
/// first, so that a name `git` would refuse beside another branch, such as `a` beside `a/b`, is
/// refused before anything else changes.
fn create_branch(repo: &Repository, name: &str, commit: &str, switch: bool) -> Result<()> {
    valid_branch_name(name)?;
    if repo.find_branch(name, BranchType::Local).is_ok() {
        bail!("a branch named '{name}' exists already");
    }
    let target = Oid::from_str(commit)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .with_context(|| format!("the commit {commit} is no longer in the repository"))?;
    repo.branch(name, &target, false)
        .with_context(|| format!("a branch named '{name}' cannot be created here"))?;
    if switch {
        switch_to_new(repo, name, &target)?;
    }
    Ok(())
}

/// Creates `branch` tracking `remote`'s branch of that name, still at `tip`, and switches to it;
/// a refusal on the way deletes the branch again.
fn track_remote(repo: &Repository, remote: &str, branch: &str, tip: &str) -> Result<()> {
    let upstream = format!("{remote}/{branch}");
    let remote_branch = repo
        .find_branch(&upstream, BranchType::Remote)
        .with_context(|| format!("there is no branch '{upstream}' any more"))?;
    at_tip(remote_branch.get(), tip, &format!("'{upstream}'"))?;
    if repo.find_branch(branch, BranchType::Local).is_ok() {
        bail!("a local branch named '{branch}' exists already: switch to it instead");
    }
    valid_branch_name(branch)?;
    let target = repo.find_commit(Oid::from_str(tip)?)?;
    let mut local = repo
        .branch(branch, &target, false)
        .with_context(|| format!("a branch named '{branch}' cannot be created here"))?;
    if let Err(e) = local.set_upstream(Some(&upstream)) {
        let _ = local.delete();
        return Err(e.into());
    }
    switch_to_new(repo, branch, &target)
}

/// Deletes the branch `name`, if it is still the one shown: at `tip`, and merged or not as
/// `merged` says. The checked-out branch is never deleted.
fn delete_branch(repo: &Repository, name: &str, tip: &str, merged: bool) -> Result<()> {
    let mut branch = branch_as_shown(repo, name, tip)?;
    if branch.is_head() {
        bail!("'{name}' is checked out: switch to another branch before deleting it");
    }
    not_checked_out_elsewhere(repo, &refname_of(&branch)?)?;
    if crate::refs::merged(repo, &branch) != merged {
        bail!("whether '{name}' is merged changed since it was shown: reload to see it as it is");
    }
    branch.delete()?;
    Ok(())
}
