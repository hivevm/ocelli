//! Verifies: ADR-0019, Q-4
//!
//! Opening a repository must not run code the repository brings. The repository here names a
//! program in every place Git would run one: every hook, `core.fsmonitor`, an external diff, a
//! textconv driver, a filter driver, and a pager. Each program would leave a marker file. Loading
//! the commit graph, the refs, the changed files, and the diffs, of a commit and of the dirty
//! working tree, leaves none.

#![cfg(unix)]

use ocelli_core::graph::build_graph;
use ocelli_core::model::WORKDIR_SHA;
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::path::{Path, PathBuf};

const HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "post-index-change",
];

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn program(marker: &Path, name: &str) -> String {
    format!("sh -c 'touch {}/{name}'", marker.display())
}

#[test]
fn reading_a_repository_runs_none_of_its_programs() {
    let root = std::env::temp_dir().join(format!("ocelli-no-repo-code-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _guard = Scratch(root.clone());
    let dir = root.join("repo");
    let markers = root.join("markers");
    std::fs::create_dir_all(&markers).unwrap();

    let repo = git2::Repository::init(&dir).unwrap();
    let sig =
        git2::Signature::new("Ada", "ada@example.org", &git2::Time::new(1_700_000_000, 0)).unwrap();
    std::fs::write(dir.join(".gitattributes"), "* diff=evil filter=evil\n").unwrap();
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(".gitattributes")).unwrap();
    index.add_path(Path::new("a.txt")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let first = repo
        .commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])
        .unwrap();
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    index.add_path(Path::new("a.txt")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = repo.find_commit(first).unwrap();
    let second = repo
        .commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
        .unwrap();

    // Name a program everywhere Git would run one, only now that the history exists.
    let hooks = root.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for hook in HOOKS {
        let path = hooks.join(hook);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch {}/hook-{hook}\n", markers.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::copy(&path, dir.join(".git/hooks").join(hook)).unwrap();
    }
    let mut cfg = repo.config().unwrap();
    cfg.set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    cfg.set_str("core.fsmonitor", &program(&markers, "fsmonitor"))
        .unwrap();
    cfg.set_str("diff.external", &program(&markers, "external-diff"))
        .unwrap();
    cfg.set_str("diff.evil.command", &program(&markers, "diff-driver"))
        .unwrap();
    cfg.set_str("diff.evil.textconv", &program(&markers, "textconv"))
        .unwrap();
    cfg.set_str("filter.evil.clean", &program(&markers, "clean"))
        .unwrap();
    cfg.set_str("filter.evil.smudge", &program(&markers, "smudge"))
        .unwrap();
    cfg.set_str("filter.evil.process", &program(&markers, "process"))
        .unwrap();
    cfg.set_str("core.pager", &program(&markers, "pager"))
        .unwrap();
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("untracked.txt"), "new\n").unwrap();

    let reader = GitReader;
    let mut quiet = |_: String| {};
    let commits = reader.load_commits(&dir, &mut quiet).unwrap();
    assert_eq!(
        build_graph(commits).nodes.len(),
        3,
        "two commits and the working directory"
    );
    reader.load_refs(&dir).unwrap();
    let head = second.to_string();
    reader.load_commit_files(&dir, &head).unwrap();
    reader.load_file_diff(&dir, &head, "a.txt").unwrap();
    reader.load_commit_files(&dir, WORKDIR_SHA).unwrap();
    reader.load_file_diff(&dir, WORKDIR_SHA, "a.txt").unwrap();

    let ran: Vec<_> = std::fs::read_dir(&markers)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(ran.is_empty(), "reading the repository ran {ran:?}");
}

#[test]
fn a_marker_program_leaves_its_marker_when_run() {
    let root = std::env::temp_dir().join(format!("ocelli-marker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _guard = Scratch(root.clone());
    std::fs::create_dir_all(&root).unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(
            program(&root, "probe")
                .trim_start_matches("sh -c ")
                .trim_matches('\''),
        )
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        root.join("probe").exists(),
        "the probe proves the markers can appear"
    );
}
