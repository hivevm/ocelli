//! Verifies: ADR-0019, Q-4
//!
//! Opening a repository must not run code the repository brings. The repository here names a
//! program in every place Git would run one, from the list both tests of Q-4 share (`programs`).
//! Each program would leave a marker file. Loading the commit graph, the refs, the changed files,
//! and the diffs, of a commit and of the dirty working tree, leaves none.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;
use ocelli_core::graph::build_graph;
use ocelli_core::model::WORKDIR_SHA;
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::path::{Path, PathBuf};
use support::Scratch;
// The programs start `git` through it.
use support::git_command;

mod programs;

#[test]
fn reading_a_repository_runs_none_of_its_programs() {
    let scratch = Scratch::new("read");
    let root = scratch.0.clone();
    let dir = root.join("repo");

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
    let markers = programs::set_everywhere(&dir, &root.join("tools"));
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("untracked.txt"), "new\n").unwrap();

    let reader = GitReader;
    let mut quiet = |_: String| {};
    let commits = reader.load_first_chunk(&dir, &mut quiet).unwrap();
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

    let ran = markers.ran();
    assert!(ran.is_empty(), "reading the repository ran {ran:?}");
}

#[test]
// Verifies: Q-4
fn every_place_of_the_list_is_set_and_each_program_leaves_its_marker() {
    let root = std::env::temp_dir().join(format!("ocelli-programs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _guard = Scratch(root.clone());
    let dir = root.join("repo");
    git2::Repository::init(&dir).unwrap();
    let markers = programs::set_everywhere(&dir, &root.join("tools"));

    let config = git2::Repository::open(&dir).unwrap().config().unwrap();
    for key in programs::PROGRAMS {
        let program = config.get_string(key).unwrap();
        let status = std::process::Command::new(&program).status().unwrap();
        assert!(status.success(), "{key}");
    }
    for (key, value) in programs::SWITCHES {
        assert_eq!(config.get_string(key).unwrap(), *value, "{key}");
    }
    let hooks = PathBuf::from(config.get_string("core.hooksPath").unwrap());
    for hook in programs::HOOKS {
        for path in [hooks.join(hook), dir.join(".git/hooks").join(hook)] {
            assert!(
                std::process::Command::new(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    let attributes = std::fs::read_to_string(dir.join(".git/info/attributes")).unwrap();
    assert_eq!(attributes, programs::ATTRIBUTES);

    let mut ran = markers.ran();
    ran.sort();
    let mut expected: Vec<String> = programs::PROGRAMS.iter().map(|k| k.to_string()).collect();
    expected.extend(programs::HOOKS.iter().map(|h| format!("hook-{h}")));
    expected.sort();
    assert_eq!(
        ran, expected,
        "every program leaves the marker named after it"
    );
}

#[test]
// Verifies: Q-4
fn the_list_holds_every_hook_git_documents() {
    let Some(documented) = programs::documented_hooks() else {
        eprintln!("git's manual is not installed here; the hooks are not compared");
        return;
    };
    let missing: Vec<_> = documented
        .iter()
        .filter(|h| !programs::HOOKS.contains(&h.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "hooks git documents that the list lacks: {missing:?}"
    );
}
