//! Verifies: ADR-0039, Q-5
//!
//! The checks that a repository lies below a server root, each way out of it a case of its own:
//! everything libgit2 reads or follows on its own (`docs/CONVENTIONS.md`, untrusted
//! configuration) built as a way out, and refused. Each case is built so that the clause it is
//! about decides it, the other clauses letting it through, so that it fails with that clause
//! removed; where two clauses each refuse a case alone, the case says so. A repository beside it
//! that could have opened does. The cases: a `.git` file or link, a git, common, or working
//! directory alone outside, `core.worktree`, a folder that is not a repository's own working
//! directory, from which a search upwards would leave the root, a git directory named in place
//! of its working directory, a bare repository, a link inside the git directory, the alternates
//! read as libgit2 reads them, and a git directory too large to walk.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;
use ocelli_git::confine::{
    Refused, WALK_CAP, check_below, common_dir, lies_below, refs_digest, still_below,
};
use std::os::unix::ffi::OsStrExt;
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

/// A linked worktree made by hand: its own git directory `gitdir`, whose common directory is
/// `common`, the git directory of `main`, checked out at `main`'s HEAD in `work`.
fn linked(main: &Path, gitdir: &Path, work: &Path) {
    std::fs::create_dir_all(gitdir).unwrap();
    std::fs::create_dir_all(work).unwrap();
    let head = git(main, &["rev-parse", "HEAD"]);
    std::fs::write(gitdir.join("HEAD"), format!("{head}\n")).unwrap();
    std::fs::write(
        gitdir.join("commondir"),
        format!("{}\n", main.join(".git").display()),
    )
    .unwrap();
    std::fs::write(
        gitdir.join("gitdir"),
        format!("{}\n", work.join(".git").display()),
    )
    .unwrap();
    std::fs::write(work.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
}

/// A server root inside a repository that encloses it, beside a repository outside it, and a
/// repository below the root that opens: the tree each way out is built on.
struct Tree {
    _scratch: Scratch,
    /// The enclosing repository.
    encl: PathBuf,
    /// The server root, resolved.
    root: PathBuf,
    /// The repository outside the root.
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
        let tree = Tree {
            _scratch: scratch,
            encl,
            root,
            outside,
        };
        assert!(
            lies_below(&tree.root.join("ok"), &tree.root),
            "a repository below the root opens"
        );
        tree
    }
}

/// That the repository at `path` is refused below `root`, when it is opened and when it is
/// checked again before a command.
fn refused(path: &Path, root: &Path) {
    assert_eq!(
        check_below(path, root, WALK_CAP),
        Err(Refused::Outside),
        "{}",
        path.display()
    );
    assert!(!still_below(path, root), "{}", path.display());
}

/// That the repository at `path` is refused when it is opened, by the walk for links, which the
/// check before a command does not repeat.
fn refused_by_the_walk(path: &Path, root: &Path) {
    assert_eq!(
        check_below(path, root, WALK_CAP),
        Err(Refused::Outside),
        "{}",
        path.display()
    );
    assert!(still_below(path, root), "only the walk refuses it");
}

#[test]
fn a_git_file_or_link_naming_a_git_directory_outside_is_refused() {
    let tree = Tree::new("confine-git-file");
    let outside_git = tree.outside.join(".git");
    // A `.git` file naming a whole repository's git directory outside.
    std::fs::create_dir_all(tree.root.join("a")).unwrap();
    std::fs::write(
        tree.root.join("a/.git"),
        format!("gitdir: {}\n", outside_git.display()),
    )
    .unwrap();
    refused(&tree.root.join("a"), &tree.root);
    // A `.git` link to it.
    std::fs::create_dir_all(tree.root.join("b")).unwrap();
    std::os::unix::fs::symlink(&outside_git, tree.root.join("b/.git")).unwrap();
    refused(&tree.root.join("b"), &tree.root);
}

#[test]
fn a_git_directory_alone_outside_is_refused() {
    let tree = Tree::new("confine-gitdir");
    // A worktree whose common directory and working directory lie inside, its own git directory
    // outside: through a `.git` file, and through a `.git` link.
    let main = tree.root.join("ok");
    linked(&main, &tree.outside.join("gd"), &tree.root.join("a"));
    refused(&tree.root.join("a"), &tree.root);
    linked(&main, &tree.outside.join("gd2"), &tree.root.join("b"));
    std::fs::remove_file(tree.root.join("b/.git")).unwrap();
    std::os::unix::fs::symlink(tree.outside.join("gd2"), tree.root.join("b/.git")).unwrap();
    refused(&tree.root.join("b"), &tree.root);
    // The same worktree with its git directory inside opens.
    linked(&main, &tree.root.join("gd"), &tree.root.join("in"));
    assert!(lies_below(&tree.root.join("in"), &tree.root));
}

#[test]
fn a_repository_whose_common_directory_alone_lies_outside_is_refused() {
    let tree = Tree::new("confine-common");
    // A worktree whose git directory and working directory lie inside, its common directory not.
    linked(&tree.outside, &tree.root.join("gd"), &tree.root.join("w"));
    refused(&tree.root.join("w"), &tree.root);
    // The same, the common directory's object store a link back below the root, so that only
    // the common directory itself leads out: the check before a command, which does not walk
    // for the link, refuses it for that alone.
    let objects = tree.outside.join(".git/objects");
    std::fs::rename(&objects, tree.root.join("objects")).unwrap();
    std::os::unix::fs::symlink(tree.root.join("objects"), &objects).unwrap();
    refused(&tree.root.join("w"), &tree.root);
}

#[test]
fn a_working_directory_outside_named_by_core_worktree_is_refused() {
    let tree = Tree::new("confine-worktree");
    // Named below the root, its working directory outside.
    let e = tree.root.join("e");
    repository(&e);
    git(
        &e,
        &["config", "core.worktree", &tree.outside.to_string_lossy()],
    );
    refused(&e, &tree.root);
    // Named at its working directory outside, its git directory below the root: the working
    // directory alone lies outside.
    let r = tree.root.join("r");
    repository(&r);
    let work = tree.outside.join("w");
    std::fs::create_dir_all(&work).unwrap();
    git(&r, &["config", "core.worktree", &work.to_string_lossy()]);
    std::fs::write(
        work.join(".git"),
        format!("gitdir: {}\n", r.join(".git").display()),
    )
    .unwrap();
    refused(&work, &tree.root);
}

#[test]
fn a_folder_without_a_repository_of_its_own_is_refused_rather_than_searched_upwards() {
    let tree = Tree::new("confine-search");
    // From each, a search upwards would find the repository enclosing the root, or the one
    // around the folder. Opening without the search, and the folder named being the working
    // directory's own, decide it together: each alone refuses it.
    std::fs::create_dir_all(tree.root.join("d/.git")).unwrap();
    std::fs::create_dir_all(tree.root.join("plain")).unwrap();
    std::fs::create_dir_all(tree.root.join("ok/src")).unwrap();
    for path in [
        tree.root.join("d"),
        tree.root.join("plain"),
        tree.root.clone(),
        tree.root.join("ok/src"),
    ] {
        refused(&path, &tree.root);
    }
    assert!(tree.encl.join(".git").is_dir(), "the enclosing repository");
}

#[test]
fn a_git_directory_named_in_place_of_its_working_directory_is_refused() {
    let tree = Tree::new("confine-named");
    // It opens without a search, as the repository whose working directory lies around it.
    refused(&tree.root.join("ok/.git"), &tree.root);
}

#[test]
fn a_bare_repository_is_refused() {
    let tree = Tree::new("confine-bare");
    let bare = tree.root.join("bare.git");
    git(
        &tree.root,
        &[
            "clone",
            "--quiet",
            "--bare",
            &tree.root.join("ok").to_string_lossy(),
            "bare.git",
        ],
    );
    refused(&bare, &tree.root);
}

#[test]
fn a_link_inside_the_git_directory_is_refused() {
    let tree = Tree::new("confine-links");
    let outside_git = tree.outside.join(".git");
    git(&tree.outside, &["gc", "--quiet"]);
    git(&tree.outside, &["pack-refs", "--all"]);
    // The object packs, a link out.
    let pack = tree.root.join("pack");
    repository(&pack);
    let packs = pack.join(".git/objects/pack");
    std::fs::remove_dir_all(&packs).unwrap();
    std::os::unix::fs::symlink(outside_git.join("objects/pack"), &packs).unwrap();
    refused_by_the_walk(&pack, &tree.root);
    // The packed refs, a link out.
    let refs = tree.root.join("refs");
    repository(&refs);
    let packed = refs.join(".git/packed-refs");
    let _ = std::fs::remove_file(&packed);
    std::os::unix::fs::symlink(outside_git.join("packed-refs"), &packed).unwrap();
    refused_by_the_walk(&refs, &tree.root);
}

#[test]
fn a_link_in_a_worktrees_own_git_directory_outside_the_common_one_is_refused() {
    let tree = Tree::new("confine-own");
    // A worktree whose git directory lies below the root but outside the common directory.
    let gitdir = tree.root.join("gd");
    let work = tree.root.join("w");
    linked(&tree.root.join("ok"), &gitdir, &work);
    assert!(
        lies_below(&work, &tree.root),
        "the worktree opens while nothing in it links"
    );
    std::os::unix::fs::symlink(std::env::temp_dir(), gitdir.join("link")).unwrap();
    refused_by_the_walk(&work, &tree.root);
}

#[test]
fn a_git_directory_larger_than_the_walk_is_refused_as_too_large() {
    let tree = Tree::new("confine-cap");
    let repo = tree.root.join("ok");
    assert_eq!(check_below(&repo, &tree.root, 5), Err(Refused::TooLarge));
    assert_eq!(check_below(&repo, &tree.root, WALK_CAP), Ok(()));
}

/// The repository `name` below the root, whose store's `alternates` file holds `text`.
fn borrowing(tree: &Tree, name: &str, text: &[u8]) -> PathBuf {
    let repo = tree.root.join(name);
    repository(&repo);
    std::fs::write(repo.join(".git/objects/info/alternates"), text).unwrap();
    repo
}

#[test]
fn object_stores_borrowed_from_outside_are_refused() {
    let tree = Tree::new("confine-stores");
    let outside_objects = tree.outside.join(".git/objects");
    // A store outside, named by an absolute line.
    let c = borrowing(
        &tree,
        "c",
        format!("{}\n", outside_objects.display()).as_bytes(),
    );
    refused(&c, &tree.root);
    // Through a store inside that borrows from outside.
    borrowing(
        &tree,
        "store",
        format!("{}\n", outside_objects.display()).as_bytes(),
    );
    let c2 = borrowing(&tree, "c2", b"../../../store/.git/objects\n");
    refused(&c2, &tree.root);
    // A store inside, named the same ways, opens.
    let fine = borrowing(&tree, "fine", b"../../../ok/.git/objects\n");
    assert!(lies_below(&fine, &tree.root));
}

#[test]
fn a_relative_alternates_line_without_the_dot_is_refused() {
    let tree = Tree::new("confine-relative");
    // libgit2 reads it against the process's working directory; a decoy lies below the root
    // where the line would lead from the store.
    let relative = borrowing(&tree, "relative", b"outside/.git/objects\n");
    std::fs::create_dir_all(relative.join(".git/objects/outside/.git/objects")).unwrap();
    refused(&relative, &tree.root);
}

#[test]
fn an_alternates_line_libgit2_ends_at_a_carriage_return_is_refused() {
    let tree = Tree::new("confine-cr");
    let outside = tree.outside.join(".git/objects");
    // Its second half outside, beside a decoy where the whole line would lead: read whole, the
    // line would be refused for its control character, so the two guards decide it together.
    for (name, line) in [
        ("cr", format!("zz\r{}", outside.display())),
        ("dot-cr", format!("./zz\r{}", outside.display())),
    ] {
        let repo = borrowing(&tree, name, format!("{line}\n").as_bytes());
        std::fs::create_dir_all(repo.join(".git/objects").join(&line)).unwrap();
        refused(&repo, &tree.root);
    }
}

#[test]
fn an_alternates_line_with_a_control_character_is_refused() {
    let tree = Tree::new("confine-control");
    // Leading to a store below the root, which the line would otherwise name.
    let tab = borrowing(&tree, "tab", b"./in\tside\n");
    std::fs::create_dir_all(tab.join(".git/objects/in\tside")).unwrap();
    refused(&tab, &tree.root);
}

#[test]
fn an_alternates_file_that_is_not_utf8_is_refused() {
    let tree = Tree::new("confine-bytes");
    let bytes = borrowing(&tree, "bytes", b"./in\xffside\n");
    // The store the line would name, where the file system takes the name: macOS's does not,
    // and the line is refused all the same.
    let _ = std::fs::create_dir_all(
        bytes
            .join(".git/objects")
            .join(std::ffi::OsStr::from_bytes(b"in\xffside")),
    );
    refused(&bytes, &tree.root);
}

#[test]
fn an_alternates_file_that_cannot_be_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let tree = Tree::new("confine-unreadable");
    let repo = borrowing(&tree, "r", b"./borrowed\n");
    std::fs::create_dir_all(repo.join(".git/objects/borrowed")).unwrap();
    assert!(
        still_below(&repo, &tree.root),
        "a readable alternates file below the root"
    );
    let file = repo.join(".git/objects/info/alternates");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&file).is_ok() {
        // Running as root reads it anyway: nothing to decide here.
        return;
    }
    assert!(!still_below(&repo, &tree.root));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn a_chain_of_alternates_is_followed_only_so_deep() {
    let tree = Tree::new("confine-chain");
    let root = &tree.root;
    let repo = root.join("ok");
    // Each store borrows the next: depth stores below the repository's own.
    let chain = |depth: usize| {
        let base = root.join(format!("stores-{depth}"));
        let mut from = repo.join(".git/objects");
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_file(from.join("info/alternates"));
        for i in 0..depth {
            let store = base.join(format!("s{i}"));
            std::fs::create_dir_all(store.join("info")).unwrap();
            std::fs::create_dir_all(from.join("info")).unwrap();
            std::fs::write(
                from.join("info/alternates"),
                format!("{}\n", store.display()),
            )
            .unwrap();
            from = store;
        }
    };
    chain(1);
    assert!(
        still_below(&repo, root),
        "one store borrowed below the root"
    );
    chain(3);
    assert!(still_below(&repo, root), "a short chain below the root");
    chain(8);
    assert!(
        !still_below(&repo, root),
        "a chain deeper than libgit2 follows is refused"
    );
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
