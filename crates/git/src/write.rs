//! The writes of the repository port, with git2 (ADR-0033). libgit2 runs no hook and no program
//! a repository's configuration names, so writing, like reading, runs no code the repository
//! brings (Q-4). Every write checks the state it expects before it changes anything.

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    BranchType, CheckoutNotificationType, Commit, ErrorCode, ObjectType, Oid, Reference,
    Repository, RepositoryState, StashApplyOptions, StashFlags, Status, TreeWalkMode,
    TreeWalkResult,
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
        Write::Fetch { remote } => fetch(&repo, remote, progress),
        Write::Pull { branch, tip } => pull(&repo, branch, tip, progress),
        Write::Push { branch, tip } => push(&repo, branch, tip, progress),
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
    let message = message.map(|m| clean_message(repo, m)).transpose()?;
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

/// `message` cleaned up as `git tag` cleans it: comment lines, surrounding blank
/// lines, and trailing spaces go, and it ends in a newline; empty where nothing is left.
fn clean_message(repo: &Repository, message: &str) -> Result<String> {
    let comment = comment_string(repo)?;
    let kept: Vec<_> = message
        .lines()
        .filter(|l| !l.starts_with(&comment))
        .collect();
    Ok(git2::message_prettify(kept.join("\n"), None)?)
}

/// Refuses `what` where the configuration's `key` asks `git` to sign it, which Ocelli cannot
/// (ADR-0033).
fn refuse_where_signing_is_asked(repo: &Repository, key: &str, what: &str) -> Result<()> {
    if repo.config()?.get_bool(key).unwrap_or(false) {
        bail!("{key} asks for {what} to be signed, which Ocelli does not do: use git for it");
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

const INDEX: Status = Status::INDEX_NEW
    .union(Status::INDEX_MODIFIED)
    .union(Status::INDEX_DELETED)
    .union(Status::INDEX_RENAMED)
    .union(Status::INDEX_TYPECHANGE);
const WORKTREE: Status = Status::WT_NEW
    .union(Status::WT_MODIFIED)
    .union(Status::WT_DELETED)
    .union(Status::WT_RENAMED)
    .union(Status::WT_TYPECHANGE);

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
        Some(head) => repo.reset_default(Some(head.as_object()), [path])?,
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
    if !matches!(
        repo.state(),
        RepositoryState::Clean | RepositoryState::Bisect
    ) {
        bail!(
            "a {} is in progress: finish it with git, which records it as it began it",
            in_progress(repo.state())
        );
    }
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
    // `git commit -m` cleans up whitespace only and keeps lines that look like comments, unless
    // `commit.cleanup` says otherwise.
    let cleanup = repo
        .config()?
        .get_string("commit.cleanup")
        .unwrap_or_default();
    let message = match cleanup.as_str() {
        "strip" => clean_message(repo, message)?,
        "verbatim" => message.to_string(),
        _ => git2::message_prettify(message, None)?,
    };
    if message.is_empty() {
        bail!("a commit needs a message");
    }
    let author = repo.signature().context(NO_IDENTITY)?;
    let parents: Vec<_> = parent.iter().collect();
    let made = repo.commit(Some("HEAD"), &author, &author, &message, &tree, &parents)?;
    Ok(made.to_string())
}

/// Refuses unless `path`'s content, where `side` of [`crate::git::shown`] reads it, is still the
/// one `expected` names (ADR-0033).
fn as_shown(
    repo: &Repository,
    path: &str,
    expected: &Option<String>,
    side: impl Fn(ocelli_core::model::Shown) -> Option<String>,
) -> Result<()> {
    if side(crate::git::shown(repo, path)) != *expected {
        bail!("{path} changed since it was shown: reload to see it as it is");
    }
    Ok(())
}

/// What a repository is in the middle of, in the words `git status` uses.
fn in_progress(state: RepositoryState) -> &'static str {
    match state {
        RepositoryState::Merge => "merge",
        RepositoryState::Revert | RepositoryState::RevertSequence => "revert",
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => "cherry-pick",
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => "git am",
        _ => "rebase",
    }
}

/// Refuses a name `git branch` refuses: one no reference can have, one that reads as an option,
/// and `HEAD`.
fn valid_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-')
        || name == "HEAD"
        || !Reference::is_valid_name(&format!("refs/heads/{name}"))
    {
        bail!("'{name}' is not a name a branch can have");
    }
    Ok(())
}

/// The local branch `name`, refused unless it still points at `tip`.
fn branch_as_shown<'r>(repo: &'r Repository, name: &str, tip: &str) -> Result<git2::Branch<'r>> {
    let branch = repo
        .find_branch(name, BranchType::Local)
        .with_context(|| format!("there is no branch named '{name}' any more"))?;
    if branch.get().target().map(|t| t.to_string()).as_deref() != Some(tip) {
        bail!("the branch '{name}' is no longer where it was shown: reload to see it as it is");
    }
    Ok(branch)
}

/// Refuses where the branch `refname` is checked out in another working tree, as `git` does.
fn not_checked_out_elsewhere(repo: &Repository, refname: &str) -> Result<()> {
    for name in repo.worktrees()?.iter().flatten() {
        let Ok(worktree) = repo.find_worktree(name) else {
            continue;
        };
        let Ok(other) = Repository::open_from_worktree(&worktree) else {
            continue;
        };
        let head = other.head().ok().and_then(|h| h.name().map(str::to_string));
        if head.as_deref() == Some(refname) {
            let short = refname.trim_start_matches("refs/heads/");
            bail!("'{short}' is checked out in the working tree '{name}'");
        }
    }
    Ok(())
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

/// Checks `target`'s tree out over the checked-out one, the uncommitted changes carried along,
/// or refuses with nothing changed, naming what is in the way: what a switch and a fast-forward
/// both do to the working directory and the index.
fn check_out(repo: &Repository, target: &Commit) -> Result<()> {
    if !matches!(
        repo.state(),
        RepositoryState::Clean | RepositoryState::Bisect
    ) {
        bail!(
            "a {} is in progress: finish it with git first",
            in_progress(repo.state())
        );
    }
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

/// The full reference name of a branch.
fn refname_of(branch: &git2::Branch) -> Result<String> {
    Ok(branch
        .get()
        .name()
        .context("a branch name git2 cannot read")?
        .to_string())
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
    if remote_branch
        .get()
        .target()
        .map(|t| t.to_string())
        .as_deref()
        != Some(tip)
    {
        bail!("'{upstream}' is no longer where it was shown: reload to see it as it is");
    }
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

/// Fetches `remote` through libgit2's own transports, as `git fetch <remote>` does, telling
/// `progress` how far it has come and stopping where it says so (ADR-0035).
fn fetch(repo: &Repository, name: &str, progress: &mut dyn FnMut(&str) -> bool) -> Result<()> {
    let mut remote = repo
        .find_remote(name)
        .with_context(|| format!("there is no remote named '{name}' any more"))?;
    let url = remote.url().unwrap_or_default().to_string();
    let transfer = Transfer::new("Fetching", name, progress);
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(transfer.callbacks());
    let fetched = remote.fetch(&[] as &[&str], Some(&mut options), None);
    drop(options);
    fetched.map_err(|e| transfer.refusal(e, &url, "fetching from"))
}

/// One transfer with a remote, a fetch or a push, through libgit2's own transports (ADR-0035):
/// what it tells `progress`, whether the developer stopped it, and how a failure reads. No
/// credential helper or command the repository's configuration names runs, and no server is
/// trusted unseen: libgit2 verifies a certificate against the system's trust store and an SSH
/// host key against the developer's `known_hosts`, since no certificate check of Ocelli's own
/// overrides it. A server that stalls is given up after [`SERVER_TIMEOUT_MS`].
struct Transfer<'p> {
    doing: &'static str,
    name: String,
    progress: std::cell::RefCell<&'p mut dyn FnMut(&str) -> bool>,
    cancelled: std::cell::Cell<bool>,
    tried: std::cell::RefCell<Tried>,
}

impl<'p> Transfer<'p> {
    fn new(doing: &'static str, name: &str, progress: &'p mut dyn FnMut(&str) -> bool) -> Self {
        set_server_timeouts();
        Self {
            doing,
            name: name.to_string(),
            progress: std::cell::RefCell::new(progress),
            cancelled: std::cell::Cell::new(false),
            tried: Default::default(),
        }
    }

    /// Tells the developer how far the transfer has come; false once they stopped it.
    fn go_on(&self, how_far: &str) -> bool {
        let go_on =
            (self.progress.borrow_mut())(&format!("{} {}: {how_far}", self.doing, self.name));
        self.cancelled.set(!go_on);
        go_on
    }

    fn callbacks(&self) -> git2::RemoteCallbacks<'_> {
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|url, user, allowed| {
            if !self.go_on("authenticating") {
                return Err(git2::Error::from_str("cancelled"));
            }
            self.tried.borrow_mut().credentials(url, user, allowed)
        });
        callbacks.sideband_progress(|_| self.go_on("the server is preparing"));
        callbacks.transfer_progress(|stats| {
            self.go_on(&format!(
                "{} of {} objects",
                stats.received_objects(),
                stats.total_objects()
            ))
        });
        // libgit2 offers no way to stop a push once its objects are sent: told, not asked.
        callbacks.push_transfer_progress(|current, total, _| {
            (self.progress.borrow_mut())(&format!(
                "{} {}: {current} of {total} objects",
                self.doing, self.name
            ));
        });
        callbacks
    }

    /// What a failed transfer tells the developer.
    fn refusal(&self, e: git2::Error, url: &str, direction: &str) -> anyhow::Error {
        let name = &self.name;
        let doing = self.doing.to_lowercase();
        if self.cancelled.get() {
            return anyhow::anyhow!("{doing} {name} was cancelled");
        }
        match e.code() {
            ErrorCode::Certificate if is_ssh(url) => anyhow::anyhow!(
                "{name}'s host key is not in your known_hosts: connect to it once with ssh, and \
                 check its fingerprint, before {direction} it from Ocelli"
            ),
            ErrorCode::Certificate => anyhow::anyhow!(
                "{name}'s server could not be verified against the system's certificates: {}",
                e.message()
            ),
            ErrorCode::Auth => anyhow::anyhow!("{name} refused the credentials: {}", e.message()),
            ErrorCode::NotFastForward => {
                anyhow::anyhow!(
                    "{name} has commits the branch does not: the remote is ahead, so pull first"
                )
            }
            _ => e.into(),
        }
    }
}

/// Pulls the checked-out branch `name`, still at `tip`, as `git pull --ff-only` does: fetches
/// its upstream's remote and fast-forwards to the upstream, the uncommitted changes carried along;
/// a branch that has diverged is refused, the fetch kept (ADR-0035).
fn pull(
    repo: &Repository,
    name: &str,
    tip: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<()> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: a pull moves the checked-out branch");
    }
    let refname = refname_of(&branch)?;
    let remote = repo
        .branch_upstream_remote(&refname)
        .ok()
        .and_then(|remote| remote.as_str().map(str::to_string))
        .with_context(|| format!("'{name}' has no upstream to pull from"))?;
    // An upstream on `.` is a local branch, with nothing to fetch.
    if remote != "." {
        fetch(repo, &remote, progress)?;
    }
    let upstream = repo
        .find_branch(name, BranchType::Local)?
        .upstream()
        .map_err(|_| anyhow::anyhow!("the upstream of '{name}' is not there to pull from"))?;
    let theirs = upstream
        .get()
        .target()
        .context("the upstream points at nothing")?;
    let ours = Oid::from_str(tip)?;
    if theirs == ours || repo.graph_descendant_of(ours, theirs)? {
        return Ok(());
    }
    if !repo.graph_descendant_of(theirs, ours)? {
        bail!(
            "'{name}' and its upstream have diverged: merge or rebase them with git; only the \
             fetch was done"
        );
    }
    let target = repo.find_commit(theirs)?;
    // The branch is locked before anything changes, and checked against `tip` again under the
    // lock: a commit made while the fetch ran is neither lost nor checked out over, and a lock
    // another program holds refuses the pull before the working directory moves.
    let mut transaction = repo.transaction()?;
    transaction
        .lock_ref(&refname)
        .with_context(|| format!("'{name}' is locked by another program: try again later"))?;
    branch_as_shown(repo, name, tip)?;
    check_out(repo, &target)?;
    transaction.set_target(&refname, theirs, None, "pull: Fast-forward")?;
    transaction.commit()?;
    Ok(())
}

/// Pushes the checked-out branch `name`, still at `tip`, as `git push` does, never by force: to
/// its upstream, or to the remote a fetch reaches without one named, under its own name, which
/// then becomes its upstream. A push the remote refuses, one that is not a fast-forward, changes
/// nothing (ADR-0035).
fn push(
    repo: &Repository,
    name: &str,
    tip: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<()> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: a push sends the checked-out branch");
    }
    let refname = refname_of(&branch)?;
    let (remote_name, destination, first) = push_destination(repo, name, &refname)?;
    let mut remote = repo
        .find_remote(&remote_name)
        .with_context(|| format!("there is no remote named '{remote_name}' any more"))?;
    let url = remote
        .pushurl()
        .or(remote.url())
        .unwrap_or_default()
        .to_string();
    let transfer = Transfer::new("Pushing", &remote_name, progress);
    let refused = std::cell::RefCell::new(None);
    let mut callbacks = transfer.callbacks();
    callbacks.push_update_reference(|_, status| {
        if let Some(why) = status {
            *refused.borrow_mut() = Some(why.to_string());
        }
        Ok(())
    });
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    // No `+`: a push that is not a fast-forward is the remote's to refuse.
    let pushed = remote.push(&[format!("{refname}:{destination}")], Some(&mut options));
    drop(options);
    pushed.map_err(|e| transfer.refusal(e, &url, "pushing to"))?;
    if let Some(why) = refused.into_inner() {
        bail!("{remote_name} refused '{name}' ({why}): the remote is ahead, so pull first");
    }
    if first {
        repo.find_branch(name, BranchType::Local)?
            .set_upstream(Some(&format!("{remote_name}/{name}")))
            .with_context(|| format!("'{name}' was pushed, but its upstream could not be set"))?;
    }
    Ok(())
}

/// Where a push of the branch `name` goes, as `git push` decides with `push.default` and the
/// branch's push remote: the remote, the reference there, and whether the branch has no upstream
/// yet, which the push then sets, as `git push -u` would. `simple`, the default, refuses an
/// upstream named differently; `nothing` and `matching` are left to `git`.
fn push_destination(
    repo: &Repository,
    name: &str,
    refname: &str,
) -> Result<(String, String, bool)> {
    let config = repo.config()?;
    let setting = |key: &str| config.get_string(key).ok().filter(|v| !v.is_empty());
    let upstream = repo
        .branch_upstream_remote(refname)
        .ok()
        .and_then(|remote| remote.as_str().map(str::to_string));
    if upstream.as_deref() == Some(".") {
        bail!("the upstream of '{name}' is a local branch, which a push does not reach");
    }
    let remote = setting(&format!("branch.{name}.pushRemote"))
        .or_else(|| setting("remote.pushDefault"))
        .or_else(|| upstream.clone())
        .or_else(|| crate::refs::default_remote(repo, &crate::refs::remote_names(repo)))
        .with_context(|| format!("there is no remote to push '{name}' to"))?;
    let merge = setting(&format!("branch.{name}.merge"));
    let to_upstream = upstream.as_deref() == Some(remote.as_str());
    let mode = setting("push.default").unwrap_or_else(|| "simple".to_string());
    let destination = match mode.as_str() {
        "nothing" => bail!("push.default is nothing: name what to push with git"),
        "matching" => bail!("push.default is matching, which pushes every branch: use git for it"),
        "current" => refname.to_string(),
        "upstream" | "tracking" => match merge.filter(|_| to_upstream) {
            Some(merge) if merge.starts_with("refs/heads/") => merge,
            _ => bail!("'{name}' has no upstream branch on {remote} to push to"),
        },
        "simple" => match merge.filter(|_| to_upstream) {
            Some(merge) if merge != refname => bail!(
                "the upstream of '{name}' is named differently ({merge}): push it with git, \
                 naming where"
            ),
            _ => refname.to_string(),
        },
        other => bail!("push.default is {other}, which Ocelli does not know: use git"),
    };
    Ok((remote, destination, upstream.is_none()))
}

/// How long libgit2 waits for a server to connect, and then for each read, before it gives up.
const SERVER_CONNECT_TIMEOUT_MS: i32 = 30_000;
const SERVER_TIMEOUT_MS: i32 = 60_000;

/// Sets libgit2's server timeouts, once for the process, before the first transfer: without them
/// a server that stalls holds a fetch, and the developer's next write, for ever.
#[allow(unsafe_code)]
fn set_server_timeouts() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: libgit2's global options are set here once, before any transfer reads them;
        // the reads of a repository do not use them.
        unsafe {
            let _ =
                git2::opts::set_server_connect_timeout_in_milliseconds(SERVER_CONNECT_TIMEOUT_MS);
            let _ = git2::opts::set_server_timeout_in_milliseconds(SERVER_TIMEOUT_MS);
        }
    });
}

/// Whether `url` reaches its remote over SSH: `ssh://`, or the scp-like `host:path`.
fn is_ssh(url: &str) -> bool {
    url.starts_with("ssh://") || url.starts_with("git+ssh://") || !url.contains("://")
}

/// The ways to authenticate, each tried once: the SSH agent, then the credential helper of the
/// developer's global and system configuration, never one the repository's own names (ADR-0035).
#[derive(Default)]
struct Tried {
    username: bool,
    agent: bool,
    helper: bool,
}

impl Tried {
    fn credentials(
        &mut self,
        url: &str,
        user: Option<&str>,
        allowed: git2::CredentialType,
    ) -> std::result::Result<git2::Cred, git2::Error> {
        use git2::CredentialType as Kind;
        let refuse = |why: &str| {
            Err(git2::Error::new(
                ErrorCode::Auth,
                git2::ErrorClass::Net,
                why,
            ))
        };
        // A line break in the address or the user name, both of which the repository's
        // configuration chooses, would add lines of its own to what a credential helper reads,
        // and could have it hand over the credential of another host (as CVE-2020-5260 did).
        if [url, user.unwrap_or_default()]
            .iter()
            .any(|s| s.contains(['\n', '\r', '\0']))
        {
            return refuse(
                "the remote's address holds a line break, which could pass a credential to another host",
            );
        }
        if allowed.contains(Kind::USERNAME) && !self.username {
            self.username = true;
            return git2::Cred::username(&user.map(str::to_string).unwrap_or_else(local_user));
        }
        if allowed.contains(Kind::SSH_KEY) && !self.agent {
            self.agent = true;
            let user = user.map(str::to_string).unwrap_or_else(local_user);
            return git2::Cred::ssh_key_from_agent(&user);
        }
        if allowed.contains(Kind::USER_PASS_PLAINTEXT) && !self.helper {
            self.helper = true;
            if let Err(why) = helper_may_answer_for(url) {
                return refuse(why);
            }
            return git2::Cred::credential_helper(&developers_config()?, url, user);
        }
        refuse(
            "neither the SSH agent nor a credential helper of your configuration gave credentials \
             the server accepted",
        )
    }
}

/// The user name ssh would use where the address names none: the local one.
fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "git".to_string())
}

/// The developer's own configuration, global and system, without the repository's: where a
/// credential helper is looked up (ADR-0035).
pub(crate) fn developers_config() -> std::result::Result<git2::Config, git2::Error> {
    git2::Config::open_default()
}

/// Refuses an address a credential helper could answer for another host than the one libgit2
/// sends its answer to. A password goes over https only. git2 tells the helper the host as the
/// `url` crate reads it, and tells it none at all for an IP address, where a helper could match any
/// credential it holds; libgit2 connects to the host after the last `@` of the address. The two
/// must name one and the same domain, written plainly.
fn helper_may_answer_for(address: &str) -> std::result::Result<(), &'static str> {
    let scheme = "https://";
    if !address
        .get(..scheme.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
    {
        return Err("credentials are sent over https only, and this remote is not");
    }
    let parsed = url::Url::parse(address).map_err(|_| "the remote's address cannot be read")?;
    let Some(url::Host::Domain(host)) = parsed.host() else {
        return Err("credentials go only to a remote named by its domain, not by an IP address");
    };
    let authority = address[scheme.len()..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let named = authority.rsplit('@').next().unwrap_or_default();
    let named = match named.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => named,
    };
    let unusual = authority.contains(['\\', ' ']) || named.contains('%');
    if unusual || !named.eq_ignore_ascii_case(host) {
        return Err("the remote's address names its host in a way that could be read two ways");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::helper_may_answer_for as asks;

    #[test]
    // Verifies: ADR-0035
    fn a_helper_answers_only_for_a_plainly_named_domain_over_https() {
        for fine in [
            "https://example.org/repo.git",
            "https://user@Example.org:8443/repo.git",
            "HTTPS://example.org/a@b/repo.git",
            // Both parsers read the host after the last `@`, so the helper is asked about the
            // very host the answer goes to.
            "https://ex%61mple.org@evil.example/repo.git",
        ] {
            assert_eq!(asks(fine), Ok(()), "{fine}");
        }
        for (refused, why) in [
            ("http://example.org/repo.git", "https only"),
            ("https://192.0.2.1/repo.git", "IP address"),
            ("https://[2001:db8::1]/repo.git", "IP address"),
            ("https://%65xample.org/repo.git", "two ways"),
            ("https://example.org\\@evil.example/repo.git", "two ways"),
            ("https://bücher.example/repo.git", "two ways"),
        ] {
            let error = asks(refused).unwrap_err();
            assert!(error.contains(why), "{refused}: {error}");
        }
    }
}
