//! Verifies: ADR-0037
//!
//! The two reads of a repository the remote server's turns take: the common directory a write's
//! turn is keyed by, one for a working directory and every worktree of it, and the digest of its
//! refs, which tells whether a refused write moved one.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;
use ocelli_git::refs::{common_dir, refs_digest};
use std::path::{Path, PathBuf};
use support::{Scratch, git};

/// A repository with one commit at `dir`.
fn repository(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet", "-b", "main"]);
    std::fs::write(dir.join("f"), "one\n").unwrap();
    git(dir, &["add", "f"]);
    git(dir, &["commit", "--quiet", "-m", "one"]);
}

/// A folder inside a repository that encloses it, with a repository `ok` below it, beside a
/// repository outside it.
struct Tree {
    _scratch: Scratch,
    /// The enclosing repository.
    encl: PathBuf,
    /// The folder, resolved: no repository of its own.
    root: PathBuf,
    /// The repository outside the folder.
    outside: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Tree {
        let scratch = Scratch::new(name);
        let base = scratch.0.canonicalize().unwrap();
        let (encl, outside) = (base.join("encl"), base.join("outside"));
        repository(&encl);
        repository(&outside);
        let root = encl.join("root");
        repository(&root.join("ok"));
        Tree {
            _scratch: scratch,
            encl,
            root,
            outside,
        }
    }
}

#[test]
// Verifies: ADR-0037
fn a_worktree_and_its_repository_share_one_common_directory_and_another_repository_does_not() {
    let tree = Tree::new("confine-shared-common");
    let main = tree.root.join("ok");
    let worktree = tree.root.join("wt");
    git(
        &main,
        &["worktree", "add", "--quiet", worktree.to_str().unwrap()],
    );
    let common = common_dir(&main).expect("the repository's common directory");
    assert_eq!(common, main.join(".git").canonicalize().unwrap());
    assert_eq!(common_dir(&worktree), Some(common.clone()));
    // Reached through a link, it is the same repository.
    std::os::unix::fs::symlink(&main, tree.root.join("via")).unwrap();
    assert_eq!(common_dir(&tree.root.join("via")), Some(common.clone()));
    assert_ne!(common_dir(&tree.outside), Some(common));
    // A folder without a repository of its own has none: the one around it is not searched for.
    assert_eq!(common_dir(&tree.root), None);
    assert!(common_dir(&tree.encl).is_some());
}

#[test]
// Verifies: ADR-0037
fn the_refs_digest_changes_with_a_ref_or_head_and_with_nothing_else() {
    let tree = Tree::new("confine-refs-digest");
    let repo = tree.root.join("ok");
    let digest = || refs_digest(&repo).expect("the repository's refs");
    let before = digest();
    assert_eq!(digest(), before, "read twice, nothing changed");
    // A change to the working directory and the index moves no ref.
    std::fs::write(repo.join("f"), "changed\n").unwrap();
    git(&repo, &["add", "f"]);
    assert_eq!(digest(), before);
    // A ref made, moved, and deleted; HEAD switched to another branch at the same commit.
    git(&repo, &["branch", "other"]);
    let made = digest();
    assert_ne!(made, before);
    git(&repo, &["commit", "--quiet", "-m", "two"]);
    let moved = digest();
    assert_ne!(moved, made);
    git(&repo, &["switch", "--quiet", "-c", "third"]);
    assert_ne!(digest(), moved);
    let switched = digest();
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    assert_ne!(digest(), switched, "HEAD names another branch");
    // A folder without a repository of its own has none.
    assert_eq!(refs_digest(&tree.root), None);
}
