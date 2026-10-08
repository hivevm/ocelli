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
use git2::build::CheckoutBuilder;
use git2::{
    BranchType, Commit, ErrorCode, ObjectType, Oid, Reference, Repository, StashApplyOptions,
    StashFlags, Status, TreeWalkMode, TreeWalkResult,
};
use ocelli_core::port::Write;
use std::path::Path;

/// Performs `write` on the repository at `repo_path`, or refuses it with nothing changed; returns
/// the commit it made, where it made one.
pub fn write(
    repo_path: &Path,
    write: &Write,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<String>> {
    let mut repo = Repository::discover(repo_path)?;
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
        } => crate::operations::resolve(&repo, path, conflict, *take),
        Write::Continue { operation, head } => {
            return crate::operations::continue_stopped(&repo, *operation, head);
        }
        Write::Abort { operation, head } => crate::operations::abort(&repo, *operation, head),
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

/// Stashes the uncommitted changes, the untracked files with them where `untracked` says so.
fn create_stash(repo: &mut Repository, message: Option<&str>, untracked: bool) -> Result<()> {
    let stasher = repo.signature().context(NO_IDENTITY)?;
    let mut flags = StashFlags::DEFAULT;
    if untracked {
        flags |= StashFlags::INCLUDE_UNTRACKED;
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    // A stash during an operation in progress would drop the operation with it, as `git stash`
    // does, a bisect aside (ADR-0033).
    nothing_in_progress(repo, true)?;
    let made = match repo.stash_save2(&stasher, message, Some(flags)) {
        Err(e) if e.code() == ErrorCode::NotFound => bail!("there is nothing to stash"),
        result => result?,
    };
    messages_as_git_writes_them(repo, made)?;
    // libgit2 leaves a staged change in the index where the working directory holds the file as
    // committed; the stash holds it now, and `git stash` resets to the checked-out commit.
    let head = repo.head()?.peel_to_commit()?;
    repo.reset(head.as_object(), git2::ResetType::Hard, None)?;
    Ok(())
}

/// The stash `made` with the messages `git stash` writes: libgit2 ends the stash commit's in a
/// line break and its index and untracked commits' in two, where `git` ends them in none and in
/// one. The commits are made again with those messages, and the stash's newest entry names the
/// new one; the entry libgit2 wrote goes first, so the stash keeps one entry for it.
fn messages_as_git_writes_them(repo: &Repository, made: Oid) -> Result<()> {
    let trimmed = |c: &Commit| -> String {
        let m = c.message().unwrap_or_default();
        m.strip_suffix('\n').unwrap_or(m).to_string()
    };
    let again = |c: &Commit| -> Result<Commit> {
        let parents: Vec<Commit> = c.parents().collect();
        let oid = repo.commit(
            None,
            &c.author(),
            &c.committer(),
            &trimmed(c),
            &c.tree()?,
            &parents.iter().collect::<Vec<_>>(),
        )?;
        Ok(repo.find_commit(oid)?)
    };
    let stash = repo.find_commit(made)?;
    let mut parents: Vec<Commit> = stash.parents().collect();
    for parent in parents.iter_mut().skip(1) {
        *parent = again(parent)?;
    }
    let new_stash = repo.commit(
        None,
        &stash.author(),
        &stash.committer(),
        &trimmed(&stash),
        &stash.tree()?,
        &parents.iter().collect::<Vec<_>>(),
    )?;
    let mut reflog = repo.reflog("refs/stash")?;
    let entry = reflog
        .get(0)
        .context("the stash has no entry for what it stashed")?;
    let said = entry.message().unwrap_or_default().to_string();
    reflog.remove(0, false)?;
    reflog.write()?;
    repo.reference("refs/stash", new_stash, true, &said)?;
    Ok(())
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
