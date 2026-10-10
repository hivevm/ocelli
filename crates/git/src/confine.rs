//! Whether a repository lies wholly below a folder (ADR-0039): the remote server opens for a
//! browser only a repository whose working directory, git directory, common directory, and object
//! stores all resolve below its server root. A repository can point outside itself through a
//! `.git` file or link, a worktree's common directory, `core.worktree`, a link anywhere inside
//! its git directory, and the stores `objects/info/alternates` borrows, and a search upwards can
//! leave the folder it started in.
//!
//! Under the remote server the adapter is confined to its server root for the whole process
//! (`confine_to`): every open then takes the folder it is given as the repository's own, never
//! searching upwards, the parent row is looked for only up to the root, and a submodule's
//! checked-out commit is read only where the submodule lies below it. The desktop application
//! and the terminal application never confine it.

use git2::{Repository, RepositoryOpenFlags};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// How deep alternates of alternates are followed: libgit2's `GIT_ALTERNATES_MAX_DEPTH`.
const ALTERNATES_DEPTH: usize = 5;

/// The folders the process is confined to, once the remote server has set one; a process runs
/// one server, and its tests several, each with a root of its own.
static ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Confines every repository the adapter opens in this process to the folders confined to, `root`
/// a resolved folder among them. Called by the remote server at its start; never undone.
pub fn confine_to(root: &Path) {
    ROOTS.write().unwrap().push(root.to_path_buf());
}

/// Whether `path` lies at or below `root`, both resolved first, links included: the one place a
/// path is compared with the root (docs/CONVENTIONS.md). On Windows libgit2 spells a path with
/// forward slashes and without the `\\?\` prefix `canonicalize` adds, so a path compared as it
/// stands lies below no resolved root there. A path or root that does not resolve, missing or
/// unreadable, lies below nothing.
#[expect(
    clippy::disallowed_methods,
    reason = "the one comparison, both sides resolved the same way first"
)]
pub fn below(path: &Path, root: &Path) -> bool {
    let (Ok(path), Ok(root)) = (path.canonicalize(), root.canonicalize()) else {
        return false;
    };
    path.starts_with(root)
}

pub(crate) fn confined() -> bool {
    !ROOTS.read().unwrap().is_empty()
}

/// Confined, the root `path` lies below, or none where it lies below none, which then reaches
/// nothing; not confined, `Ok(None)`.
pub(crate) fn root_of(path: &Path) -> Result<Option<PathBuf>, ()> {
    let roots = ROOTS.read().unwrap();
    if roots.is_empty() {
        return Ok(None);
    }
    roots
        .iter()
        .find(|root| below(path, root))
        .cloned()
        .map(Some)
        .ok_or(())
}

/// Opens the repository at `path`: found by searching upwards, as `git` does, or, confined,
/// at `path` alone, so that a `.git` that vanished after the check fails instead of leading to
/// a repository around the root.
pub(crate) fn open(path: &Path) -> Result<Repository, git2::Error> {
    if confined() {
        Repository::open_ext(
            path,
            RepositoryOpenFlags::NO_SEARCH,
            std::iter::empty::<&OsStr>(),
        )
    } else {
        Repository::discover(path)
    }
}

/// Whether libgit2 reaches `url` over the network rather than through its local transport, which
/// opens a repository on this machine in-process: an address with a network scheme, or one that
/// reads as `host:path` as ssh's does where no folder of that name exists, which libgit2 would
/// open locally first. Anything else, a path or a `file://` address, is local.
fn over_the_network(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let schemes = [
        "http://",
        "https://",
        "ssh://",
        "ssh+git://",
        "git+ssh://",
        "git://",
    ];
    if schemes.iter().any(|scheme| lower.starts_with(scheme)) {
        return true;
    }
    let scp_like = !url.contains("://")
        && url
            .split_once(':')
            .is_some_and(|(host, _)| host.len() > 1 && !host.contains(['/', '\\']));
    scp_like && !Path::new(url).exists()
}

/// Confined, refuses a remote's address `url` that libgit2 would reach through its local
/// transport unless it is an absolute path, or a `file://` address of one, that resolves below
/// the root the repository at `repo_path` lies below: a fetch from a repository outside the root
/// would copy its objects inside, where the browser reads them, and a push would write it, which
/// ADR-0037 rules out however a repository inside points outside. An address over the network is
/// the server account's to reach (ADR-0037). Not confined, every address passes.
pub(crate) fn remote_within_root(repo_path: &Path, name: &str, url: &str) -> anyhow::Result<()> {
    let root = match root_of(repo_path) {
        Ok(None) => return Ok(()),
        Ok(Some(root)) => root,
        Err(()) => anyhow::bail!("the repository lies outside the server root"),
    };
    if over_the_network(url) {
        return Ok(());
    }
    let lower = url.to_ascii_lowercase();
    let path = if lower.starts_with("file://") {
        &url["file://".len()..]
    } else {
        url
    };
    // A path libgit2 would decode or resolve against the process's folder is not followed.
    let path = Path::new(path);
    if url.contains('%') || !path.is_absolute() || !below(path, &root) {
        anyhow::bail!(
            "{name} is a repository on this machine outside the server root, which the remote \
             server does not reach: fetch or push it with git"
        );
    }
    Ok(())
}

/// Why a repository is not opened for a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// It reaches outside the root, or is no repository's working directory itself.
    Outside,
    /// Its git directory holds more entries than the check walks.
    TooLarge,
}

/// The most entries of a repository's git and common directories the check walks for links by
/// default: past them the repository is refused rather than checked in part.
pub const WALK_CAP: usize = 500_000;

/// Whether the repository at `path`, opened without searching upwards, lies wholly below `root`,
/// a resolved folder: its git directory, its common directory, its working directory, and every
/// object store its alternates name, links resolved, with no link anywhere inside its git and
/// common directories. A folder that is no repository's working directory itself, a bare
/// repository, and a store that does not resolve lie below nothing. Made when a repository is
/// opened; a link made inside it afterwards, and a hard link, are not seen. It reads the file
/// system: a caller on an async runtime runs it on the blocking pool.
pub fn lies_below(path: &Path, root: &Path) -> bool {
    check_below(path, root, WALK_CAP).is_ok()
}

/// `lies_below`, walking at most `cap` entries of the git and common directories, and saying
/// why it refuses.
pub fn check_below(path: &Path, root: &Path, cap: usize) -> Result<(), Refused> {
    check(path, root, Some(cap))
}

/// The check made again before each command on a repository that passed `lies_below`: its
/// directories and its alternates, without walking its git directory for links again.
pub fn still_below(path: &Path, root: &Path) -> bool {
    check(path, root, None).is_ok()
}

fn check(path: &Path, root: &Path, walk: Option<usize>) -> Result<(), Refused> {
    let repo = Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&OsStr>(),
    )
    .map_err(|_| Refused::Outside)?;
    let workdir = repo.workdir().ok_or(Refused::Outside)?;
    // The folder named is the working directory's own, so no folder inside a repository opens
    // the repository around it.
    let named = path.canonicalize().ok();
    if named.is_none() || named != workdir.canonicalize().ok() {
        return Err(Refused::Outside);
    }
    if !(below(repo.path(), root) && below(repo.commondir(), root) && below(workdir, root)) {
        return Err(Refused::Outside);
    }
    if let Some(cap) = walk {
        let mut budget = cap;
        no_link_below(repo.commondir(), &mut budget)?;
        // A linked worktree's git directory lies inside the common directory, walked already.
        if !below(repo.path(), repo.commondir()) {
            no_link_below(repo.path(), &mut budget)?;
        }
    }
    if !stores_below(&repo.commondir().join("objects"), root) {
        return Err(Refused::Outside);
    }
    Ok(())
}

/// That nothing below `dir`, `dir` itself resolved, is a link: a link inside a git directory is
/// read through by libgit2 wherever it leads. Each entry seen spends one of `budget`.
fn no_link_below(dir: &Path, budget: &mut usize) -> Result<(), Refused> {
    let dir = dir.canonicalize().map_err(|_| Refused::Outside)?;
    let mut next = vec![dir];
    while let Some(dir) = next.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|_| Refused::Outside)?;
        for entry in entries {
            *budget = budget.checked_sub(1).ok_or(Refused::TooLarge)?;
            let kind = entry
                .and_then(|e| e.file_type().map(|k| (e, k)))
                .map_err(|_| Refused::Outside)?;
            if kind.1.is_symlink() {
                return Err(Refused::Outside);
            }
            if kind.1.is_dir() {
                next.push(kind.0.path());
            }
        }
    }
    Ok(())
}

/// The stores an `alternates` file of the store `objects` names, read as libgit2 reads it: lines
/// split at `\r` and `\n`, empty ones and comments skipped, a line starting with `.` relative to
/// `objects`, any other taken as it stands. `None` where a line is relative without the `.`,
/// which libgit2 reads against the process's working directory, or holds a control character.
fn alternates(objects: &Path, text: &str) -> Option<Vec<PathBuf>> {
    let mut stores = Vec::new();
    for line in text.split(['\r', '\n']) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.chars().any(char::is_control) {
            return None;
        }
        if line.starts_with('.') {
            stores.push(objects.join(line));
        } else if Path::new(line).is_absolute() {
            stores.push(PathBuf::from(line));
        } else {
            return None;
        }
    }
    Some(stores)
}

/// Whether `objects` and every store its alternates name, theirs in turn, resolve below `root`.
fn stores_below(objects: &Path, root: &Path) -> bool {
    let mut seen = HashSet::new();
    let mut next = vec![(objects.to_path_buf(), 0)];
    while let Some((store, depth)) = next.pop() {
        let Ok(store) = store.canonicalize() else {
            return false;
        };
        if !below(&store, root) {
            return false;
        }
        if !seen.insert(store.clone()) {
            continue;
        }
        let file = store.join("info/alternates");
        let text = match std::fs::read(&file) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => return false,
            },
            // No alternates file borrows nothing; one that cannot be read is refused.
            Err(_) if !file.exists() => continue,
            Err(_) => return false,
        };
        let Some(stores) = alternates(&store, &text) else {
            return false;
        };
        if !stores.is_empty() && depth >= ALTERNATES_DEPTH {
            return false;
        }
        next.extend(stores.into_iter().map(|s| (s, depth + 1)));
    }
    true
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "a test sets its folders up by hand, the way a developer would"
)]
mod below_tests {
    use super::below;
    use std::path::PathBuf;

    /// A scratch folder holding `root` and `outside` beside it, named for the test and the
    /// process, and removed when the test ends; unresolved, as `temp_dir` spells it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ocelli-confine-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("root/inside")).unwrap();
            std::fs::create_dir_all(dir.join("outside")).unwrap();
            Scratch(dir)
        }

        fn root(&self) -> PathBuf {
            self.0.join("root")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    // Verifies: ADR-0039
    fn the_root_itself_lies_below_it_however_either_is_spelled() {
        let scratch = Scratch::new("itself");
        let root = scratch.root();
        let resolved = root.canonicalize().unwrap();
        assert!(below(&root, &root));
        assert!(below(&root, &resolved));
        assert!(below(&resolved, &root));
    }

    #[test]
    // Verifies: ADR-0039
    fn a_path_with_forward_slashes_is_resolved_before_it_is_compared() {
        let scratch = Scratch::new("slashes");
        // As libgit2 spells a path on Windows: forward slashes, no `\\?\` prefix.
        let spelled = scratch
            .root()
            .join("inside")
            .to_string_lossy()
            .replace('\\', "/");
        let root = scratch.root().canonicalize().unwrap();
        assert!(below(std::path::Path::new(&spelled), &root));
        let outside = scratch
            .0
            .join("outside")
            .to_string_lossy()
            .replace('\\', "/");
        assert!(!below(std::path::Path::new(&outside), &root));
    }

    #[test]
    // Verifies: ADR-0039
    fn a_path_with_dot_dot_is_resolved_before_it_is_compared() {
        let scratch = Scratch::new("dotdot");
        let root = scratch.root();
        assert!(below(&root.join("inside/.."), &root));
        assert!(!below(&root.join("inside/../../outside"), &root));
        assert!(!below(&root.join(".."), &root));
    }

    #[test]
    // Verifies: ADR-0039
    fn a_missing_path_lies_below_nothing() {
        let scratch = Scratch::new("missing");
        let root = scratch.root();
        assert!(!below(&root.join("missing"), &root));
        assert!(!below(&root, &scratch.0.join("missing")));
    }

    #[cfg(unix)]
    #[test]
    // Verifies: ADR-0039
    fn a_link_out_of_the_root_lies_outside_it() {
        let scratch = Scratch::new("link");
        let root = scratch.root();
        std::os::unix::fs::symlink(scratch.0.join("outside"), root.join("out")).unwrap();
        std::os::unix::fs::symlink(root.join("inside"), root.join("in")).unwrap();
        assert!(!below(&root.join("out"), &root));
        assert!(below(&root.join("in"), &root));
    }
}

// An absolute path is spelled for unix here.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    // Verifies: ADR-0039
    fn alternates_are_read_as_libgit2_reads_them() {
        let objects = Path::new("/r/.git/objects");
        assert_eq!(
            alternates(objects, "# a comment\n\n../../o/objects\r\n/abs/objects\n"),
            Some(vec![
                objects.join("../../o/objects"),
                PathBuf::from("/abs/objects")
            ])
        );
        // `\r` ends a line for libgit2 as `\n` does.
        assert_eq!(
            alternates(objects, "zz\r/abs/objects"),
            None,
            "zz is relative without the dot"
        );
        assert_eq!(alternates(objects, "outside/.git/objects"), None);
        assert_eq!(alternates(objects, "/abs/\tobjects"), None);
        assert_eq!(alternates(objects, ""), Some(vec![]));
    }

    #[test]
    // Verifies: ADR-0039, ADR-0037
    fn an_address_is_over_the_network_only_with_a_network_scheme_or_as_ssh_reads_host_and_path() {
        for network in [
            "https://example.org/r.git",
            "HTTP://example.org/r.git",
            "ssh://git@example.org/r.git",
            "git+ssh://example.org/r",
            "ssh+git://example.org/r",
            "git://example.org/r",
            "git@example.org:r.git",
            "example.org:r.git",
        ] {
            assert!(over_the_network(network), "{network}");
        }
        for local in [
            "/srv/remote.git",
            "file:///srv/remote.git",
            "FILE:///srv/remote.git",
            "../remote.git",
            "remote.git",
            "C:\\remote.git",
            "C:/remote.git",
            "./a:b",
            "/srv/a:b",
            "",
        ] {
            assert!(!over_the_network(local), "{local}");
        }
    }
}
