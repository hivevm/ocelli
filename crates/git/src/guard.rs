//! What a write checks before it changes anything: that the repository still holds what the
//! developer saw, that nothing is in progress that the write would mix with, that an identity
//! is configured and no signature is asked for (ADR-0031). Each check is written once here, for
//! the local writes, the transfers, and the operations alike, with the wrappers through which
//! alone a git2 method that would panic on a string is called (`clippy.toml`).

use crate::refs::operation_of;
use anyhow::{Context, Result, bail};
use git2::{
    BranchType, ConfigEntry, ErrorCode, Oid, Reference, Repository, RepositoryState, Transaction,
};
use ocelli_core::port::{Conflict, Write};
use std::path::{Path, PathBuf};

/// The refusal where no identity is configured to write with.
pub(crate) const NO_IDENTITY: &str =
    "no identity is configured to write with: set user.name and user.email";

/// Refuses `what` where the configuration's `key` asks `git` to sign it, which Ocelli cannot
/// (ADR-0031).
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
/// one `expected` names (ADR-0031).
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

/// Refuses a string holding a NUL, which git2 would panic on, naming it as `what`.
fn nul_free(value: &str, what: &str) -> Result<()> {
    if value.contains('\0') {
        bail!("the {what} holds a NUL character, which git does not take");
    }
    Ok(())
}

/// Locks `refname` in `transaction`. git2's `Transaction::lock_ref` panics on a NUL: it is
/// called only here, after the NUL is refused.
#[expect(
    clippy::disallowed_methods,
    reason = "the one call, a name with a NUL refused before git2 sees it"
)]
pub(crate) fn lock_ref(transaction: &mut Transaction, refname: &str) -> Result<()> {
    nul_free(refname, "reference name")?;
    Ok(transaction.lock_ref(refname)?)
}

/// Points the locked `refname` of `transaction` at `target`, logged with `message`. git2's
/// `Transaction::set_target` panics on a NUL: it is called only here, after the NUL is refused.
#[expect(
    clippy::disallowed_methods,
    reason = "the one call, a name or message with a NUL refused before git2 sees it"
)]
pub(crate) fn set_target(
    transaction: &mut Transaction,
    refname: &str,
    target: Oid,
    message: &str,
) -> Result<()> {
    nul_free(refname, "reference name")?;
    nul_free(message, "reflog message")?;
    Ok(transaction.set_target(refname, target, None, message)?)
}

/// The stashes, newest first, each with its message and commit, read from the stash's reflog as
/// libgit2's `git_stash_foreach` reads them. git2's `Repository::stash_foreach` panics on a
/// message that is not UTF-8, which a repository may hold: a message is read here lossily.
pub(crate) fn stashes(repo: &Repository) -> Result<Vec<(String, Oid)>> {
    match repo.find_reference("refs/stash") {
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
        found => found?,
    };
    Ok(repo
        .reflog("refs/stash")?
        .iter()
        .map(|entry| {
            let message = entry.message_bytes().unwrap_or_default();
            (
                String::from_utf8_lossy(message).into_owned(),
                entry.id_new(),
            )
        })
        .collect())
}

/// The value of a configuration entry, none where it is not UTF-8, refused for a key written
/// without one, as `git` refuses it for a key that takes a string. git2's `ConfigEntry::value`
/// panics on such a key, which a repository's configuration may hold.
#[expect(
    clippy::disallowed_methods,
    reason = "the one call, a key without a value refused before git2 is asked"
)]
pub(crate) fn config_value<'e>(entry: &'e ConfigEntry) -> Result<Option<&'e str>> {
    if !entry.has_value() {
        let key = String::from_utf8_lossy(entry.name_bytes());
        bail!("missing value for '{key}' in the configuration, which git refuses too");
    }
    Ok(entry.value())
}

/// The path of the file `file` of a diff names. git2's `DiffFile::path` unwraps it as UTF-8 on
/// Windows, where a tree may hold a name that is not: there it is read lossily, a name Windows
/// could not hold anyway, and elsewhere as the bytes it is.
pub(crate) fn diff_path(file: &git2::DiffFile) -> Option<PathBuf> {
    file.path_bytes().map(bytes_to_path)
}

/// The path a tree, the index, or a diff names in `bytes`, to hand back to libgit2 or to the file
/// system: the bytes it is, and on Windows, whose paths are UTF-16, and on macOS, whose APFS
/// takes UTF-8 alone, only where it is UTF-8. libgit2 takes a path's bytes, and one rebuilt from
/// a lossy reading names another file or none (a checkout's exact pathspec matches nothing, and
/// the file is not written while `HEAD` moves).
pub(crate) fn exact_path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(any(windows, target_os = "macos"))]
    if std::str::from_utf8(bytes).is_err() {
        bail!(
            "{} is not UTF-8, a name this system cannot give a file",
            String::from_utf8_lossy(bytes)
        );
    }
    Ok(bytes_to_path(bytes))
}

/// Refuses a path a caller gives that is the lossy reading of a name that is not UTF-8, one the
/// index or the working directory holds: the caller was shown a name that cannot be shown
/// exactly, and acting on the lossy one would act on another file or none.
pub(crate) fn named_exactly(repo: &Repository, path: &str) -> Result<()> {
    if !path.contains(char::REPLACEMENT_CHARACTER) {
        return Ok(());
    }
    let lossy = |bytes: &[u8]| {
        std::str::from_utf8(bytes).is_err() && String::from_utf8_lossy(bytes) == path
    };
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(true);
    if repo.index()?.iter().any(|e| lossy(&e.path))
        || repo
            .statuses(Some(&mut options))?
            .iter()
            .any(|e| lossy(e.path_bytes()))
    {
        bail!(
            "{path} stands for a name that is not UTF-8, which cannot be shown exactly: use git \
             for this file"
        );
    }
    Ok(())
}

/// The bytes of the repository's path `path`, as the index holds it: those [`exact_path`] read
/// it from, and on Windows its UTF-8 with forward slashes.
pub(crate) fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().replace('\\', "/").into_bytes()
    }
}

/// The path the bytes `bytes` spell, lossily where a path is UTF-16, on Windows.
pub(crate) fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// The submodules of `repo` with their paths, as configured. On Windows, where git2's
/// `Submodule::path` unwraps the path as UTF-8 and has no form in bytes, none where a path
/// libgit2 could give one is not UTF-8. libgit2 reads its sources again after the check: a
/// file another program changes between the two is not seen by it.
pub(crate) fn submodules(repo: &Repository) -> Vec<(git2::Submodule<'_>, PathBuf)> {
    #[cfg(windows)]
    if !submodule_paths_are_utf8(repo) {
        return Vec::new();
    }
    let Ok(submodules) = repo.submodules() else {
        return Vec::new();
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "git2 reads the path in bytes but on Windows, where every path libgit2 takes a \
                  submodule's from is UTF-8, as checked above"
    )]
    submodules
        .into_iter()
        .map(|submodule| {
            let path = submodule.path().to_path_buf();
            (submodule, path)
        })
        .collect()
}

/// Whether every place libgit2 takes a submodule's path from holds UTF-8 there: the names and
/// paths of the submodules in `.gitmodules` and in the repository's configuration, and the
/// path of every submodule entry of the index and of the checked-out commit.
#[cfg(windows)]
#[expect(
    clippy::disallowed_methods,
    reason = "a value is asked for only once the entry is known to hold one"
)]
fn submodule_paths_are_utf8(repo: &Repository) -> bool {
    let config_utf8 = |config: Result<git2::Config, git2::Error>| {
        let Ok(config) = config.and_then(|mut c| c.snapshot()) else {
            return false;
        };
        let Ok(entries) = config.entries(Some("^submodule\\.")) else {
            return false;
        };
        let mut utf8 = true;
        let walked = entries.for_each(|entry| {
            utf8 &= entry.name().is_some() && (!entry.has_value() || entry.value().is_some());
        });
        walked.is_ok() && utf8
    };
    let gitmodules = repo
        .workdir()
        .map(|w| w.join(".gitmodules"))
        .filter(|p| p.exists());
    if gitmodules.is_some_and(|p| !config_utf8(git2::Config::open(&p))) {
        return false;
    }
    if !config_utf8(repo.config()) {
        return false;
    }
    const GITLINK: u32 = 0o160000;
    if let Ok(index) = repo.index()
        && index
            .iter()
            .any(|e| e.mode == GITLINK && std::str::from_utf8(&e.path).is_err())
    {
        return false;
    }
    let Ok(head) = repo.head().and_then(|h| h.peel_to_tree()) else {
        return true;
    };
    let mut utf8 = true;
    let walked = head.walk(git2::TreeWalkMode::PreOrder, |_, entry| {
        if entry.filemode() as u32 == GITLINK && std::str::from_utf8(entry.name_bytes()).is_err() {
            utf8 = false;
        }
        git2::TreeWalkResult::Ok
    });
    walked.is_ok() && utf8
}

/// The path of the working tree `worktree`, named `name`. On Windows, where git2's
/// `Worktree::path` unwraps it as UTF-8 and has no form in bytes, none where the file libgit2
/// read it from is not UTF-8; the file is read again here, after libgit2 read it, so one another
/// program changes between the two is not seen as libgit2 saw it.
pub(crate) fn worktree_path(
    repo: &Repository,
    name: &str,
    worktree: &git2::Worktree,
) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let link = repo.commondir().join("worktrees").join(name).join("gitdir");
        std::str::from_utf8(&std::fs::read(link).ok()?).ok()?;
    }
    #[cfg(not(windows))]
    let _ = (repo, name);
    #[expect(
        clippy::disallowed_methods,
        reason = "git2 reads the path in bytes but on Windows, where the file libgit2 reads it \
                  from, below a directory it resolved, is UTF-8, as checked above"
    )]
    Some(worktree.path().to_path_buf())
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
        Write::SaveMerge {
            path,
            conflict:
                Conflict {
                    path: conflicted,
                    ancestor,
                    ours,
                    theirs,
                },
            opened,
            result,
        } => {
            add("path", path);
            add("conflict.path", conflicted);
            for (field, id) in [("ancestor", ancestor), ("ours", ours), ("theirs", theirs)] {
                id.iter().for_each(|id| add(field, id));
            }
            add("opened", opened);
            add("result", result);
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
    use super::{lock_ref, repository_path, set_target, valid_ref_name};

    #[test]
    // Verifies: ADR-0031
    fn a_reference_name_with_a_nul_is_no_name_and_reaches_no_panic() {
        assert!(valid_ref_name("refs/heads/topic"));
        for name in ["refs/heads/a\0b", "\0", "refs/tags/v1\0"] {
            assert!(!valid_ref_name(name), "{name:?}");
        }
        assert!(!valid_ref_name("refs/heads/a..b"));
    }

    /// A scratch directory of this process, removed when it goes, a failed test's too.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        #[expect(
            clippy::disallowed_methods,
            reason = "a test removes the repository it made"
        )]
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    // Verifies: ADR-0031
    fn a_transaction_refuses_a_nul_before_git2_sees_it() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("ocelli-guard-nul-{}", std::process::id())));
        let repo = git2::Repository::init(&scratch.0).unwrap();
        let mut transaction = repo.transaction().unwrap();
        let held = lock_ref(&mut transaction, "refs/heads/a\0b").unwrap_err();
        assert!(held.to_string().contains("NUL"), "{held}");
        let id = git2::Oid::zero();
        for (name, message) in [("refs/heads/a\0b", "m"), ("refs/heads/a", "m\0")] {
            let set = set_target(&mut transaction, name, id, message).unwrap_err();
            assert!(set.to_string().contains("NUL"), "{set}");
        }
    }

    #[test]
    // Verifies: ADR-0031, ADR-0037
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
