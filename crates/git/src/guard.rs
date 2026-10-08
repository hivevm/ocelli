//! What a write checks before it changes anything: that the repository still holds what the
//! developer saw, that nothing is in progress that the write would mix with, that an identity
//! is configured and no signature is asked for (ADR-0033). Each check is written once here, for
//! the local writes, the transfers, and the operations alike.

use anyhow::{Context, Result, bail};
use git2::{BranchType, Reference, Repository, RepositoryState};
use ocelli_core::model::Operation;

/// The refusal where no identity is configured to write with.
pub(crate) const NO_IDENTITY: &str =
    "no identity is configured to write with: set user.name and user.email";

/// Refuses `what` where the configuration's `key` asks `git` to sign it, which Ocelli cannot
/// (ADR-0033).
pub(crate) fn refuse_where_signing_is_asked(
    repo: &Repository,
    key: &str,
    what: &str,
) -> Result<()> {
    if repo.config()?.get_bool(key).unwrap_or(false) {
        bail!("{key} asks for {what} to be signed, which Ocelli does not do: use git for it");
    }
    Ok(())
}

/// Refuses unless `path`'s content, where `side` of [`crate::git::shown`] reads it, is still the
/// one `expected` names (ADR-0033).
pub(crate) fn as_shown(
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

/// The operation in progress `state` names, where Ocelli can continue or abort it (ADR-0033).
pub(crate) fn operation_of(state: RepositoryState) -> Option<Operation> {
    match state {
        RepositoryState::Merge => Some(Operation::Merge),
        RepositoryState::Revert | RepositoryState::RevertSequence => Some(Operation::Revert),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => {
            Some(Operation::CherryPick)
        }
        RepositoryState::Rebase
        | RepositoryState::RebaseInteractive
        | RepositoryState::RebaseMerge => Some(Operation::Rebase),
        _ => None,
    }
}

/// Refuses while the repository is in the middle of anything, a bisect too unless `bisect`
/// allows it, naming it in the words `git status` uses, and saying where it is finished.
pub(crate) fn nothing_in_progress(repo: &Repository, bisect: bool) -> Result<()> {
    let state = repo.state();
    let name = match (state, operation_of(state)) {
        (RepositoryState::Clean, _) => return Ok(()),
        (RepositoryState::Bisect, _) if bisect => return Ok(()),
        (_, Some(operation)) => {
            bail!(
                "a {} is in progress: finish it first, continuing or aborting it here or with git",
                operation.name()
            )
        }
        (RepositoryState::Bisect, None) => "bisect",
        (_, None) => "git am",
    };
    bail!("a {name} is in progress: finish it with git first")
}

/// Refuses unless `reference`, which the developer saw as `what`, still points at `tip`.
pub(crate) fn at_tip(reference: &git2::Reference, tip: &str, what: &str) -> Result<()> {
    if reference.target().map(|t| t.to_string()).as_deref() != Some(tip) {
        bail!("{what} is no longer where it was shown: reload to see it as it is");
    }
    Ok(())
}

/// The local branch `name`, refused unless it still points at `tip`.
pub(crate) fn branch_as_shown<'r>(
    repo: &'r Repository,
    name: &str,
    tip: &str,
) -> Result<git2::Branch<'r>> {
    let branch = repo
        .find_branch(name, BranchType::Local)
        .with_context(|| format!("there is no branch named '{name}' any more"))?;
    at_tip(branch.get(), tip, &format!("the branch '{name}'"))?;
    Ok(branch)
}

/// Refuses where the branch `refname` is checked out in another working tree, as `git` does.
pub(crate) fn not_checked_out_elsewhere(repo: &Repository, refname: &str) -> Result<()> {
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

/// The full reference name of a branch.
pub(crate) fn refname_of(branch: &git2::Branch) -> Result<String> {
    Ok(branch
        .get()
        .name()
        .context("a branch name git2 cannot read")?
        .to_string())
}

/// Refuses a name `git branch` refuses: one no reference can have, one that reads as an option,
/// and `HEAD`.
pub(crate) fn valid_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-')
        || name == "HEAD"
        || !Reference::is_valid_name(&format!("refs/heads/{name}"))
    {
        bail!("'{name}' is not a name a branch can have");
    }
    Ok(())
}
