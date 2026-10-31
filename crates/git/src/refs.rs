// Verifies: ADR-0027
//! Reads what the repository sidebar lists (ADR-0027, ADR-0028): local branches,
//! remote branches grouped by their remote, working trees, tags, submodules, and
//! the repository one level up.
//!
//! This is a read of repository *state*, not of history — nothing here produces
//! nodes or edges. Branches, worktrees and tags resolve to a commit SHA, which is
//! all the sidebar needs to navigate to it. Submodules and the parent resolve to
//! a *path* instead: they name another repository, which the sidebar opens rather
//! than navigates within.
//!
//! A single unreadable ref must not blank the whole panel, so each loader skips
//! entries it cannot resolve instead of failing the read. Only opening the
//! repository itself is fatal, and that fails the history load as well.
//!
//! For the remote server's turns it also reads which repository a folder belongs to, by its
//! common directory, and a digest of where its refs point (ADR-0035).

use anyhow::Result;
use git2::{BranchType, ObjectType, Repository, RepositoryOpenFlags, RepositoryState};
use ocelli_core::model::{
    Conflict, InProgress, LocalBranch, Operation, ParentKind, ParentRepo, Remote, RemoteBranch,
    RepoRefs, Submodule, SubmoduleState, Tag, Worktree,
};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The tip of the main line (ADR-0014): the commit of the local branch `origin/HEAD` names, else
/// of `main`, else of `master`, else the checked-out commit; none where `HEAD` does not resolve
/// either. A remote branch alone does not make a main line.
pub fn load_main_tip(repo_path: &Path) -> Result<Option<String>> {
    let repo = crate::confine::open(repo_path)?;
    let local = |name: &str| {
        repo.find_branch(name, BranchType::Local)
            .ok()
            .and_then(|b| b.get().peel_to_commit().ok())
            .map(|c| c.id().to_string())
    };
    let named_by_origin = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.symbolic_target().map(str::to_string))
        .and_then(|target| {
            target
                .strip_prefix("refs/remotes/origin/")
                .map(str::to_string)
        });
    let tip = named_by_origin
        .and_then(|name| local(&name))
        .or_else(|| local("main"))
        .or_else(|| local("master"))
        .or_else(|| {
            repo.head()
                .ok()
                .and_then(|h| h.peel_to_commit().ok())
                .map(|c| c.id().to_string())
        });
    Ok(tip)
}

/// Loads every ref category the sidebar shows, in one pass over the repository.
pub fn load_repo_refs(repo_path: &Path) -> Result<RepoRefs> {
    let repo = crate::confine::open(repo_path)?;
    let remote_names = remote_names(&repo);

    Ok(RepoRefs {
        locals: load_locals(&repo),
        remotes: load_remotes(&repo, &remote_names),
        worktrees: load_worktrees(&repo),
        tags: load_tags(&repo),
        submodules: load_submodules(&repo),
        parent: find_parent(&repo),
        default_remote: default_remote(&repo, &remote_names),
        in_progress: stopped(&repo),
    })
}

/// The remote of the checked-out branch's upstream, else `origin`, else the only remote.
pub(crate) fn default_remote(repo: &Repository, names: &[String]) -> Option<String> {
    repo.head()
        .ok()
        .filter(|head| head.is_branch())
        .and_then(|head| head.name().map(str::to_string))
        .and_then(|branch| repo.branch_upstream_remote(&branch).ok())
        .and_then(|remote| remote.as_str().map(str::to_string))
        // An upstream on `.` or on a URL names no remote a fetch could reach.
        .filter(|remote| names.contains(remote))
        .or_else(|| names.iter().find(|n| *n == "origin").cloned())
        .or_else(|| (names.len() == 1).then(|| names[0].clone()))
}

/// The first seven characters of a SHA, matching the graph's short form.
fn short(sha: &str) -> String {
    sha[..sha.len().min(7)].to_string()
}

pub(crate) fn remote_names(repo: &Repository) -> Vec<String> {
    repo.remotes()
        .map(|names| names.iter().flatten().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Splits a remote-tracking branch name into its remote and the branch below it.
///
/// Matching goes against the repository's configured remotes, longest name
/// first, because a remote name may itself contain a slash — splitting at the
/// first slash would then cut in the wrong place. Falls back to the first
/// slash for a branch whose remote is no longer configured, so a stale
/// tracking ref still lands somewhere sensible.
fn split_remote_branch<'a>(full_name: &'a str, remotes: &[String]) -> Option<(String, &'a str)> {
    let mut candidates: Vec<&String> = remotes.iter().collect();
    candidates.sort_by_key(|r| std::cmp::Reverse(r.len()));

    for remote in candidates {
        if let Some(rest) = full_name.strip_prefix(remote.as_str())
            && let Some(branch) = rest.strip_prefix('/')
            && !branch.is_empty()
        {
            return Some((remote.clone(), branch));
        }
    }

    let (remote, branch) = full_name.split_once('/')?;
    if remote.is_empty() || branch.is_empty() {
        return None;
    }
    Some((remote.to_string(), branch))
}

/// Whether every commit of `branch` is in its upstream, or in `HEAD` where it has none or that
/// does not resolve, as `git branch -d` decides.
pub(crate) fn merged(repo: &Repository, branch: &git2::Branch) -> bool {
    let Some(tip) = branch.get().target() else {
        return false;
    };
    let into = branch
        .upstream()
        .ok()
        .and_then(|u| u.get().target())
        .or_else(|| repo.head().ok().and_then(|h| h.target()));
    into.is_some_and(|into| into == tip || repo.graph_descendant_of(into, tip).unwrap_or(false))
}

fn load_locals(repo: &Repository) -> Vec<LocalBranch> {
    let Ok(branches) = repo.branches(Some(BranchType::Local)) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (branch, _) in branches.flatten() {
        let Ok(Some(name)) = branch.name() else {
            continue;
        };
        let name = name.to_string();
        let Some(oid) = branch.get().target() else {
            continue;
        };

        // An upstream that is configured but not fetched yet has no target, in
        // which case the branch is neither ahead nor behind anything known.
        let (upstream, ahead, behind) = match branch.upstream() {
            Ok(upstream) => {
                let upstream_name = upstream.name().ok().flatten().map(str::to_string);
                let (ahead, behind) = upstream
                    .get()
                    .target()
                    .and_then(|target| repo.graph_ahead_behind(oid, target).ok())
                    .unwrap_or((0, 0));
                (upstream_name, ahead, behind)
            }
            Err(_) => (None, 0, 0),
        };

        let sha = oid.to_string();
        out.push(LocalBranch {
            merged: merged(repo, &branch),
            name,
            short_sha: short(&sha),
            sha,
            is_head: branch.is_head(),
            upstream,
            ahead,
            behind,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn load_remotes(repo: &Repository, remote_names: &[String]) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = remote_names
        .iter()
        .map(|name| Remote {
            name: name.clone(),
            branches: Vec::new(),
        })
        .collect();

    let Ok(branches) = repo.branches(Some(BranchType::Remote)) else {
        return remotes;
    };

    for (branch, _) in branches.flatten() {
        let Ok(Some(full_name)) = branch.name() else {
            continue;
        };
        let Some((remote_name, branch_name)) = split_remote_branch(full_name, remote_names) else {
            continue;
        };
        // `origin/HEAD` is a symbolic pointer at the remote's default branch,
        // not a branch of its own — it would appear twice otherwise.
        if branch_name == "HEAD" {
            continue;
        }
        let Some(oid) = branch.get().target() else {
            continue;
        };

        let sha = oid.to_string();
        let entry = RemoteBranch {
            name: branch_name.to_string(),
            short_sha: short(&sha),
            sha,
        };

        match remotes.iter_mut().find(|r| r.name == remote_name) {
            Some(remote) => remote.branches.push(entry),
            // A tracking ref whose remote is gone from the config still exists
            // on disk; showing it under its own heading beats dropping it.
            None => remotes.push(Remote {
                name: remote_name,
                branches: vec![entry],
            }),
        }
    }

    for remote in &mut remotes {
        remote.branches.sort_by(|a, b| a.name.cmp(&b.name));
    }
    remotes.sort_by(|a, b| a.name.cmp(&b.name));
    remotes
}

/// Resolves a repository's `HEAD` to its branch name (when not detached) and
/// the commit it points at.
fn head_of(repo: &Repository) -> (Option<String>, String) {
    let Ok(head) = repo.head() else {
        return (None, String::new());
    };
    let branch = if head.is_branch() {
        head.shorthand().map(str::to_string)
    } else {
        None
    };
    let sha = head
        .peel(ObjectType::Commit)
        .map(|obj| obj.id().to_string())
        .unwrap_or_default();
    (branch, sha)
}

/// The main working tree's repository handle.
///
/// Opened through `commondir` when a linked worktree is being viewed: `repo`
/// then describes *that* worktree, and its `workdir` is not the main one. The
/// shared git directory is the only route back, which is also what makes the
/// parent row work for a worktree.
fn main_repo(repo: &Repository) -> Option<Repository> {
    if repo.is_worktree() {
        Repository::open(repo.commondir()).ok()
    } else {
        None
    }
}

/// The label a path gets in the sidebar: its last component.
fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// Compares two paths for "the same directory", canonicalizing when the
/// filesystem allows it so that a symlinked or `..`-containing path still
/// matches. A path that cannot be canonicalized (a submodule that was never
/// checked out) falls back to a plain comparison.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn load_worktrees(repo: &Repository) -> Vec<Worktree> {
    let mut out = Vec::new();
    let main = main_repo(repo);
    // Viewed from a linked worktree, `repo` is that worktree — the main working
    // tree has to be read through the shared git directory, or it would be
    // listed as the linked one and appear twice.
    let main_ref = main.as_ref().unwrap_or(repo);
    let current_workdir = repo.workdir().map(Path::to_path_buf);

    // `Repository::worktrees` reports linked worktrees only, so the main
    // working tree has to be added explicitly. A bare repository has none.
    if let Some(workdir) = main_ref.workdir() {
        let (branch, sha) = head_of(main_ref);
        out.push(Worktree {
            name: dir_name(workdir),
            path: workdir.to_string_lossy().to_string(),
            branch,
            short_sha: short(&sha),
            sha,
            is_main: true,
            is_current: !repo.is_worktree(),
        });
    }

    let Ok(names) = repo.worktrees() else {
        return out;
    };
    for name in names.iter().flatten() {
        let Ok(worktree) = repo.find_worktree(name) else {
            continue;
        };
        // A worktree whose directory was deleted without `git worktree prune`
        // is still registered; opening it fails, and it carries no commit.
        let (branch, sha) = match Repository::open_from_worktree(&worktree) {
            Ok(wt_repo) => head_of(&wt_repo),
            Err(_) => (None, String::new()),
        };

        let Some(path) = crate::guard::worktree_path(repo, name, &worktree) else {
            continue;
        };
        let is_current = current_workdir
            .as_deref()
            .is_some_and(|current| same_dir(current, &path));

        out.push(Worktree {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            branch,
            short_sha: short(&sha),
            sha,
            is_main: false,
            is_current,
        });
    }

    out
}

/// Derives a submodule's state from the two commit ids git tracks for it: the
/// one the superproject pins, and the one its working copy actually has.
///
/// Kept separate from the `git2` walk so the rule — which is the whole meaning
/// of the row's badge — is testable without a repository on disk.
fn submodule_state(pinned: Option<&str>, checked_out: Option<&str>) -> SubmoduleState {
    match (pinned, checked_out) {
        (_, None) => SubmoduleState::Uninitialized,
        (Some(pinned), Some(checked_out)) if pinned != checked_out => SubmoduleState::Modified,
        _ => SubmoduleState::InSync,
    }
}

/// Reads the submodules the superproject configures.
///
/// Every id here belongs to the submodule's own object database, never to this
/// repository's, so none of them is a commit the graph could reveal — the path
/// is what the sidebar acts on (ADR-0028).
fn load_submodules(repo: &Repository) -> Vec<Submodule> {
    let Some(workdir) = repo.workdir() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (submodule, path) in crate::guard::submodules(repo) {
        let name = submodule
            .name()
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        // `workdir_id` is only set once the submodule has a working copy; an
        // absent one is exactly what "uninitialized" means.
        // Confined, a submodule is opened only where it lies below the root (ADR-0037); one
        // that reaches out of it reads as one without a working copy, a row without its switch.
        let full = workdir.join(&path);
        let readable = match crate::confine::root_of(&full) {
            Ok(None) => true,
            Ok(Some(root)) => crate::confine::lies_below(&full, &root),
            Err(()) => false,
        };
        let checked_out = readable
            .then(|| submodule.workdir_id())
            .flatten()
            .map(|oid| oid.to_string());
        let pinned = submodule.head_id().map(|oid| oid.to_string());
        let state = submodule_state(pinned.as_deref(), checked_out.as_deref());

        let full_path = workdir.join(&path);
        let sha = pinned.unwrap_or_default();
        out.push(Submodule {
            name,
            path: path.to_string_lossy().to_string(),
            workdir: match state {
                SubmoduleState::Uninitialized => String::new(),
                _ => full_path.to_string_lossy().to_string(),
            },
            url: submodule.url().map(str::to_string),
            short_sha: short(&sha),
            sha,
            checked_out_short_sha: checked_out.as_deref().map(short),
            state,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Finds the repository one level up, so the sidebar offers a way back out
/// (ADR-0028).
///
/// Two cases, in order: a linked worktree belongs to a main working tree, found
/// through the shared git directory; a submodule's working copy sits inside its
/// superproject, found by discovering upwards from the directory above it.
///
/// The upward search alone is not enough — any repository may happen to lie
/// inside another one. The candidate only counts as a parent when one of *its*
/// submodules resolves to this working directory, which is the same question
/// `git rev-parse --show-superproject-working-tree` answers.
fn find_parent(repo: &Repository) -> Option<ParentRepo> {
    if let Some(main) = main_repo(repo)
        && let Some(workdir) = main.workdir()
    {
        return Some(ParentRepo {
            kind: ParentKind::MainWorktree,
            name: dir_name(workdir),
            path: workdir.to_string_lossy().to_string(),
        });
    }

    let workdir = repo.workdir()?;
    let above = workdir.parent()?;
    let candidate = match crate::confine::root_of(workdir).ok()? {
        None => Repository::discover(above).ok()?,
        // Confined, the search upwards stops at the root, and a superproject that reaches out of
        // it is none (ADR-0037).
        Some(root) => {
            let root = root.as_path();
            // The folders above, links resolved, walked up to the root.
            let above = above.canonicalize().ok()?;
            let found = above
                .ancestors()
                .take_while(|dir| crate::confine::below(dir, root))
                .find_map(|dir| {
                    Repository::open_ext(
                        dir,
                        git2::RepositoryOpenFlags::NO_SEARCH,
                        std::iter::empty::<&std::ffi::OsStr>(),
                    )
                    .ok()
                })?;
            let found_workdir = found.workdir()?.to_path_buf();
            if !crate::confine::lies_below(&found_workdir, root) {
                return None;
            }
            found
        }
    };
    let candidate_workdir = candidate.workdir()?.to_path_buf();

    let configures_us = crate::guard::submodules(&candidate)
        .iter()
        .any(|(_, path)| same_dir(&candidate_workdir.join(path), workdir));

    configures_us.then(|| ParentRepo {
        kind: ParentKind::Superproject,
        name: dir_name(&candidate_workdir),
        path: candidate_workdir.to_string_lossy().to_string(),
    })
}

fn load_tags(repo: &Repository) -> Vec<Tag> {
    let Ok(names) = repo.tag_names(None) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for name in names.iter().flatten() {
        let Ok(reference) = repo.find_reference(&format!("refs/tags/{}", name)) else {
            continue;
        };
        // Peeling resolves an annotated tag to its commit; a tag on a blob or
        // tree has no commit to navigate to and is skipped.
        let Ok(object) = reference.peel(ObjectType::Commit) else {
            continue;
        };

        let sha = object.id().to_string();
        out.push(Tag {
            name: name.to_string(),
            short_sha: short(&sha),
            sha,
            annotated: reference.peel(ObjectType::Tag).is_ok(),
        });
    }

    out
}

/// The operation in progress `state` names, where Ocelli can continue or abort it (ADR-0031).
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

/// The operation stopped in the repository, as `git status` reads it, with its conflicted files
/// (ADR-0031).
pub(crate) fn stopped(repo: &Repository) -> Option<InProgress> {
    let operation = operation_of(repo.state())?;
    let index = repo.index().ok()?;
    let conflicts = conflicts_of(&index);
    let head = repo.head().ok()?.peel_to_commit().ok()?.id().to_string();
    Some(InProgress {
        operation,
        conflicts,
        head,
    })
}

pub(crate) fn conflicts_of(index: &git2::Index) -> Vec<Conflict> {
    let Ok(conflicts) = index.conflicts() else {
        return Vec::new();
    };
    let mut out: Vec<Conflict> = conflicts
        .flatten()
        .filter_map(|c| {
            let any = c
                .our
                .as_ref()
                .or(c.their.as_ref())
                .or(c.ancestor.as_ref())?;
            let id = |e: &Option<git2::IndexEntry>| e.as_ref().map(|e| e.id.to_string());
            Some(Conflict {
                path: String::from_utf8_lossy(&any.path).into_owned(),
                ancestor: id(&c.ancestor),
                ours: id(&c.our),
                theirs: id(&c.their),
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The common directory of the repository at `path`, opened without searching upwards, resolved,
/// links included: one for a working directory and every worktree that belongs to it, which the
/// remote server orders its tabs' writes by (ADR-0035). `None` where no repository opens there or
/// its common directory does not resolve.
pub fn common_dir(path: &Path) -> Option<PathBuf> {
    let repo = Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&OsStr>(),
    )
    .ok()?;
    repo.commondir().canonicalize().ok()
}

/// A digest of where `HEAD` and every ref of the repository at `path` point, opened without
/// searching upwards: the remote server compares the digest taken before a write with the one after, to
/// tell its other tabs on the repository to load again where a refused write moved a ref, as a
/// pull's fetch does before its fast-forward is refused (ADR-0035). `None` where no repository
/// opens there or its refs cannot be listed.
pub fn refs_digest(path: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let repo = Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&OsStr>(),
    )
    .ok()?;
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    let mut each = |reference: &git2::Reference| {
        reference.name_bytes().hash(&mut digest);
        reference
            .target()
            .map(|oid| oid.as_bytes().to_vec())
            .hash(&mut digest);
        reference.symbolic_target_bytes().hash(&mut digest);
    };
    if let Ok(head) = repo.find_reference("HEAD") {
        each(&head);
    }
    for reference in repo.references().ok()? {
        let reference = reference.ok()?;
        // Ocelli's own scratch references come and go with a push's read-back, moving nothing.
        if !reference
            .name_bytes()
            .starts_with(crate::remote::OWN_REFS.as_bytes())
        {
            each(&reference);
        }
    }
    Some(digest.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remotes(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// The common case: one remote, branch name below it kept intact.
    #[test]
    fn remote_branch_splits_off_its_remote() {
        let configured = remotes(&["origin"]);
        assert_eq!(
            split_remote_branch("origin/main", &configured),
            Some(("origin".to_string(), "main"))
        );
        // A slash inside the branch name belongs to the branch, not the remote.
        assert_eq!(
            split_remote_branch("origin/feature/sidebar", &configured),
            Some(("origin".to_string(), "feature/sidebar"))
        );
    }

    /// A remote name may contain a slash, so the longest configured remote wins
    /// over a split at the first slash.
    #[test]
    fn longest_configured_remote_wins() {
        let configured = remotes(&["team", "team/fork"]);
        assert_eq!(
            split_remote_branch("team/fork/main", &configured),
            Some(("team/fork".to_string(), "main"))
        );
    }

    /// A tracking ref whose remote was removed from the config still resolves,
    /// so it can be shown instead of silently dropped.
    #[test]
    fn unknown_remote_falls_back_to_the_first_slash() {
        assert_eq!(
            split_remote_branch("gone/main", &remotes(&["origin"])),
            Some(("gone".to_string(), "main"))
        );
    }

    /// Names that cannot name both a remote and a branch are rejected.
    #[test]
    fn names_without_both_parts_are_rejected() {
        let configured = remotes(&["origin"]);
        assert_eq!(split_remote_branch("main", &configured), None);
        assert_eq!(split_remote_branch("origin/", &configured), None);
        assert_eq!(split_remote_branch("/main", &configured), None);
    }

    /// No working copy means uninitialized, whatever the superproject pins —
    /// the viewer never clones one to find out (ADR-0025).
    #[test]
    fn a_submodule_without_a_working_copy_is_uninitialized() {
        assert_eq!(
            submodule_state(Some("abc"), None),
            SubmoduleState::Uninitialized
        );
        assert_eq!(submodule_state(None, None), SubmoduleState::Uninitialized);
    }

    /// A working copy on a different commit than the pin is the "modified"
    /// state git itself reports for a submodule.
    #[test]
    fn a_working_copy_off_the_pin_is_modified() {
        assert_eq!(
            submodule_state(Some("abc"), Some("def")),
            SubmoduleState::Modified
        );
        assert_eq!(
            submodule_state(Some("abc"), Some("abc")),
            SubmoduleState::InSync
        );
    }

    /// A superproject that records no commit for the submodule has nothing for
    /// the working copy to deviate from, so the row must not claim it drifted.
    #[test]
    fn no_pin_recorded_is_not_modified() {
        assert_eq!(submodule_state(None, Some("abc")), SubmoduleState::InSync);
    }

    /// Path comparison drives both the parent check and the current-worktree
    /// mark, so a trailing `..` must not make two names for one directory look
    /// like two directories.
    #[test]
    fn same_dir_sees_through_relative_segments() {
        let here = std::env::current_dir().expect("a working directory");
        let detour = here.join("src").join("..");
        assert!(same_dir(&here, &detour));
        assert!(!same_dir(&here, &here.join("src")));
    }
}
