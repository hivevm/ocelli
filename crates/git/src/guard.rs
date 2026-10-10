//! What a write checks before it changes anything: that the repository still holds what the
//! developer saw, that nothing is in progress that the write would mix with, that an identity
//! is configured and no signature is asked for (ADR-0033). Each check is written once here, for
//! the local writes, the transfers, and the operations alike.

use crate::refs::operation_of;
use anyhow::{Context, Result, bail};
use git2::{BranchType, Reference, Repository, RepositoryState};
use ocelli_core::port::{Conflict, Write};

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

/// Refuses a `path` the caller gave that names no file of the working directory as the index
/// names one: empty, holding a NUL, absolute, with a `.` or `..` or a drive among its parts, or
/// passing through a `.git` as a file system that ignores case, trailing dots and spaces, or
/// short names reads it. git2 panics on some of these where it reads the index, and a `.git`
/// leads into the repository's own files; nothing of git2 sees the path before this.
pub(crate) fn repository_path(path: &str) -> Result<()> {
    // Split as the index spells a path, each part one plain name as this system reads it too, so
    // that neither `dir/./f`, which `components` would tidy away, nor a drive or a `\` on Windows
    // slips through.
    let plain = !path.contains('\0')
        && path.split('/').all(|part| {
            let mut parts = std::path::Path::new(part).components();
            matches!(
                (parts.next(), parts.next()),
                (Some(std::path::Component::Normal(name)), None)
                    if name == part && !names_git(part)
            )
        });
    if !plain {
        bail!("{path:?} names no file of the working directory");
    }
    Ok(())
}

/// Whether `name` reads as `.git` where case, trailing dots and spaces, or an 8.3 short name are
/// ignored, as git's own `verify_path` reads it.
fn names_git(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let name = name.trim_end_matches(['.', ' ']);
    name == ".git" || name == "git~1"
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

/// Whether `refname` is a name a reference can have. git2's `Reference::is_valid_name` panics on
/// a NUL, which a caller's name may hold: it is asked only here, after the NUL is refused.
#[expect(
    clippy::disallowed_methods,
    reason = "the one call, a name with a NUL refused before git2 sees it"
)]
pub(crate) fn valid_ref_name(refname: &str) -> bool {
    !refname.contains('\0') && Reference::is_valid_name(refname)
}

/// Every string a caller gives in `write`, by the field's name: names, ids, messages, paths, a
/// remote's name. Each field is named, so that a field added to a write is a string the check
/// below sees or a compile error here.
fn strings_of<'w>(write: &'w Write) -> Vec<(&'static str, &'w str)> {
    let mut found = Vec::new();
    let mut add = |field: &'static str, value: &'w str| found.push((field, value));
    match write {
        Write::CreateTag {
            name,
            commit,
            message,
        } => {
            add("name", name);
            add("commit", commit);
            message.iter().for_each(|m| add("message", m));
        }
        Write::DeleteTag {
            name,
            commit,
            annotated: _,
        } => {
            add("name", name);
            add("commit", commit);
        }
        Write::CreateStash {
            message,
            untracked: _,
        } => message.iter().for_each(|m| add("message", m)),
        Write::ApplyStash {
            index: _,
            commit,
            drop: _,
        }
        | Write::DropStash { index: _, commit } => add("commit", commit),
        Write::StageFile { path, shown } | Write::UnstageFile { path, shown } => {
            add("path", path);
            shown.iter().for_each(|s| add("shown", s));
        }
        Write::Commit { message, head } => {
            add("message", message);
            head.iter().for_each(|h| add("head", h));
        }
        Write::CreateBranch {
            name,
            commit,
            switch: _,
        } => {
            add("name", name);
            add("commit", commit);
        }
        Write::SwitchBranch { name, tip } => {
            add("name", name);
            add("tip", tip);
        }
        Write::Detach { commit } => add("commit", commit),
        Write::TrackRemote {
            remote,
            branch,
            tip,
        } => {
            add("remote", remote);
            add("branch", branch);
            add("tip", tip);
        }
        Write::DeleteBranch {
            name,
            tip,
            merged: _,
        } => {
            add("name", name);
            add("tip", tip);
        }
        Write::Fetch { remote } => add("remote", remote),
        Write::Pull { branch, tip } | Write::Push { branch, tip } => {
            add("branch", branch);
            add("tip", tip);
        }
        Write::Merge {
            branch,
            tip,
            commit,
            name,
        } => {
            add("branch", branch);
            add("tip", tip);
            add("commit", commit);
            name.iter().for_each(|n| add("name", n));
        }
        Write::Rebase { branch, tip, onto } => {
            add("branch", branch);
            add("tip", tip);
            add("onto", onto);
        }
        Write::CherryPick {
            branch,
            tip,
            commit,
        }
        | Write::Revert {
            branch,
            tip,
            commit,
        } => {
            add("branch", branch);
            add("tip", tip);
            add("commit", commit);
        }
        Write::Reset {
            branch,
            tip,
            commit,
            mode: _,
        } => {
            add("branch", branch);
            add("tip", tip);
            add("commit", commit);
        }
        Write::Resolve {
            path,
            conflict:
                Conflict {
                    path: conflicted,
                    ancestor,
                    ours,
                    theirs,
                },
            take: _,
        } => {
            add("path", path);
            add("conflict.path", conflicted);
            for (field, id) in [("ancestor", ancestor), ("ours", ours), ("theirs", theirs)] {
                id.iter().for_each(|id| add(field, id));
            }
        }
        Write::Continue { operation: _, head } | Write::Abort { operation: _, head } => {
            add("head", head)
        }
    }
    found
}

/// Refuses a write any string of which holds a NUL, before git2 sees it: git2 turns a string into
/// a C string with `CString::new(…).unwrap()` in places, which panics on one, and a release build
/// aborts on a panic, taking every tab of the remote server with it; nothing `git` takes holds one.
pub(crate) fn no_nul(write: &Write) -> Result<()> {
    if let Some((field, _)) = strings_of(write).iter().find(|(_, s)| s.contains('\0')) {
        bail!("the {field} given holds a NUL character, which git does not take");
    }
    Ok(())
}

/// Refuses a name `git branch` refuses: one no reference can have, one that reads as an option,
/// and `HEAD`.
pub(crate) fn valid_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-') || name == "HEAD" || !valid_ref_name(&format!("refs/heads/{name}")) {
        bail!("'{name}' is not a name a branch can have");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{repository_path, valid_ref_name};

    #[test]
    // Verifies: ADR-0033
    fn a_reference_name_with_a_nul_is_no_name_and_reaches_no_panic() {
        assert!(valid_ref_name("refs/heads/topic"));
        for name in ["refs/heads/a\0b", "\0", "refs/tags/v1\0"] {
            assert!(!valid_ref_name(name), "{name:?}");
        }
        assert!(!valid_ref_name("refs/heads/a..b"));
    }

    #[test]
    // Verifies: ADR-0033, ADR-0039
    fn a_path_is_taken_only_as_plain_names_below_the_working_directory_outside_any_git() {
        for plain in [
            "f",
            "dir/f",
            "a b/c.txt",
            ".gitignore",
            ".gitmodules",
            "x.git",
            "git",
        ] {
            assert!(repository_path(plain).is_ok(), "{plain:?}");
        }
        for wrong in [
            "",
            ".",
            "./f",
            "../x",
            "dir/../f",
            "dir/./f",
            "dir//f",
            "dir/",
            "/abs",
            "/",
            "f\0g",
            ".git",
            ".git/config",
            "sub/.git/config",
            ".GIT/config",
            ".Git",
            ".git./config",
            ".git /config",
            "GIT~1/config",
        ] {
            assert!(repository_path(wrong).is_err(), "{wrong:?}");
        }
    }
}
