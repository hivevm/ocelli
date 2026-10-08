#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
// Verifies: ADR-0027
//! Fixture tests for the repository-state read behind the sidebar (ADR-0026,
//! ADR-0027): submodules, working trees, and the way back to the parent.
//!
//! These cannot be unit tests — a submodule and a linked worktree only exist as
//! directories, gitlinks, and a shared git directory on disk. The fixtures are
//! built with `git2` itself rather than the `git` CLI or a temp-directory crate,
//! so the test adds no dependency the crate does not already have.

use git2::{Repository, Signature};
use ocelli_core::model::{ParentKind, SubmoduleState};
use ocelli_git::refs::load_repo_refs;
use std::fs;
use std::path::{Path, PathBuf};

mod support;
use support::Scratch;

/// Compares a path the reader produced with the fixture's own, through the
/// filesystem, so a symlinked temp directory does not fail the comparison.
fn same_path(reported: &str, expected: &Path) -> bool {
    match (Path::new(reported).canonicalize(), expected.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => Path::new(reported) == expected,
    }
}

/// Initializes a repository with one commit, so it has a `HEAD` to check out.
fn repo_with_a_commit(path: &Path, file: &str) -> Repository {
    let repo = Repository::init(path).expect("an initialized repository");
    fs::write(path.join(file), "content\n").expect("a file to commit");
    commit_all(&repo, "initial commit");
    repo
}

/// Stages the whole working directory and commits it onto `HEAD`.
fn commit_all(repo: &Repository, message: &str) {
    let mut index = repo.index().expect("an index");
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .expect("staged files");
    index.write().expect("a written index");
    let tree = repo
        .find_tree(index.write_tree().expect("a tree"))
        .expect("the written tree");
    let who = Signature::now("Fixture", "fixture@example.invalid").expect("a signature");
    let parents = match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
        Some(parent) => vec![parent],
        None => vec![],
    };
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(Some("HEAD"), &who, &who, message, &tree, &parent_refs)
        .expect("a commit");
}

/// A superproject with `vendor/child` as a checked-out submodule, cloned from a
/// second repository in the same scratch directory. Local paths only — the
/// clone is a filesystem copy, never egress (ADR-0023).
fn superproject_with_submodule(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let child_path = scratch.0.join("child");
    repo_with_a_commit(&child_path, "child.txt");

    let super_path = scratch.0.join("super");
    let super_repo = repo_with_a_commit(&super_path, "super.txt");

    let url = child_path.to_string_lossy().to_string();
    let mut submodule = super_repo
        .submodule(&url, Path::new("vendor/child"), true)
        .expect("a configured submodule");
    submodule.clone(None).expect("a cloned submodule");
    submodule.add_finalize().expect("the submodule staged");
    commit_all(&super_repo, "add the submodule");

    let child_workdir = super_path.join("vendor").join("child");
    (super_path, child_workdir)
}

/// The superproject lists its submodule, pinned and in sync, and points at a
/// working copy the sidebar can open.
#[test]
fn a_superproject_lists_its_submodule_with_a_path_to_open() {
    let scratch = Scratch::new("lists");
    let (super_path, child_workdir) = superproject_with_submodule(&scratch);

    let refs = load_repo_refs(&super_path).expect("the repository state");

    assert_eq!(refs.submodules.len(), 1, "one configured submodule");
    let sub = &refs.submodules[0];
    assert_eq!(sub.path, "vendor/child");
    assert_eq!(sub.state, SubmoduleState::InSync);
    assert!(
        Path::new(&sub.workdir).ends_with("vendor/child"),
        "an absolute working copy path to open, got {:?}",
        sub.workdir
    );
    assert_eq!(sub.short_sha.len(), 7, "the pinned commit, shortened");
    assert_eq!(
        sub.checked_out_short_sha.as_deref(),
        Some(&sub.short_sha[..])
    );

    // The pinned commit lives in the submodule's object database, so it is not
    // a commit of this repository — which is why the row opens rather than jumps.
    let child = Repository::open(&child_workdir).expect("the submodule repository");
    assert!(
        child
            .find_commit(child.head().unwrap().target().unwrap())
            .is_ok(),
        "the pin resolves in the submodule, not in the superproject"
    );
    assert!(
        Repository::open(&super_path)
            .unwrap()
            .find_commit(git2::Oid::from_str(&sub.sha).unwrap())
            .is_err(),
        "the superproject must not contain the submodule's commit"
    );

    // The superproject is the top: nothing above it to navigate back to.
    assert!(refs.parent.is_none(), "a superproject has no parent");
}

/// A fresh clone has the submodule configured but never checked out. That row
/// has nothing to open and must say so rather than pointing at a path that does
/// not exist — the viewer will not clone it (ADR-0023).
#[test]
fn a_never_initialized_submodule_offers_no_path() {
    let scratch = Scratch::new("uninit");
    let (super_path, _) = superproject_with_submodule(&scratch);

    let clone_path = scratch.0.join("clone");
    Repository::clone(&super_path.to_string_lossy(), &clone_path).expect("a local clone");

    let refs = load_repo_refs(&clone_path).expect("the repository state");
    let sub = refs.submodules.first().expect("the configured submodule");

    assert_eq!(sub.state, SubmoduleState::Uninitialized);
    assert!(sub.workdir.is_empty(), "no working copy to open");
    assert!(
        sub.checked_out_short_sha.is_none(),
        "nothing is checked out"
    );
    assert!(!sub.sha.is_empty(), "the pin is still recorded");
}

/// Opened on its own, the submodule finds its way back to the superproject.
#[test]
fn a_submodule_names_its_superproject_as_parent() {
    let scratch = Scratch::new("parent");
    let (super_path, child_workdir) = superproject_with_submodule(&scratch);

    let refs = load_repo_refs(&child_workdir).expect("the repository state");
    let parent = refs.parent.expect("a superproject to return to");

    assert_eq!(parent.kind, ParentKind::Superproject);
    assert!(
        Path::new(&parent.path).ends_with("super"),
        "the parent points at the superproject, got {:?}",
        parent.path
    );
    assert_eq!(parent.name, "super");
    assert!(
        same_path(&parent.path, &super_path),
        "the exact superproject path"
    );
}

/// A repository that merely sits inside another one is not a submodule of it,
/// and must not be presented as having a parent.
#[test]
fn a_nested_but_unconfigured_repository_has_no_parent() {
    let scratch = Scratch::new("nested");
    let outer = scratch.0.join("outer");
    repo_with_a_commit(&outer, "outer.txt");

    let inner = outer.join("inner");
    fs::create_dir_all(&inner).expect("the nested directory");
    repo_with_a_commit(&inner, "inner.txt");

    let refs = load_repo_refs(&inner).expect("the repository state");
    assert!(
        refs.parent.is_none(),
        "only a configured submodule has a superproject"
    );
}

/// From a linked worktree the main working tree is the parent, both trees are
/// listed once, and exactly the one being viewed is marked current.
#[test]
fn a_linked_worktree_sees_the_main_tree_as_parent() {
    let scratch = Scratch::new("worktree");
    let main_path = scratch.0.join("main");
    let repo = repo_with_a_commit(&main_path, "main.txt");

    let linked_path = scratch.0.join("linked");
    repo.worktree("linked", &linked_path, None)
        .expect("a linked worktree");

    // Seen from the main working tree.
    let from_main = load_repo_refs(&main_path).expect("the repository state");
    assert!(from_main.parent.is_none(), "the main tree is the top");
    assert_eq!(
        from_main.worktrees.len(),
        2,
        "the main tree and the linked one"
    );
    let main_row = from_main
        .worktrees
        .iter()
        .find(|w| w.is_main)
        .expect("a main row");
    assert!(main_row.is_current, "the main tree is the one being viewed");
    assert_eq!(
        from_main.worktrees.iter().filter(|w| w.is_current).count(),
        1,
        "exactly one working tree is current"
    );

    // Seen from the linked worktree: the same two rows, the mark moved, and a
    // parent to go back to.
    let from_linked = load_repo_refs(&linked_path).expect("the repository state");
    let parent = from_linked.parent.expect("the main tree to return to");
    assert_eq!(parent.kind, ParentKind::MainWorktree);
    assert!(
        Path::new(&parent.path).ends_with("main"),
        "the parent is the main working tree, got {:?}",
        parent.path
    );
    assert_eq!(
        from_linked.worktrees.len(),
        2,
        "the main tree is read through the shared git directory, not doubled"
    );
    let linked_row = from_linked
        .worktrees
        .iter()
        .find(|w| w.name == "linked")
        .expect("the linked row");
    assert!(
        linked_row.is_current,
        "the linked tree is the one being viewed"
    );
    assert!(
        !from_linked
            .worktrees
            .iter()
            .find(|w| w.is_main)
            .unwrap()
            .is_current,
        "the main tree is not current while a linked one is viewed"
    );
}
