//! Verifies: ADR-0008, ADR-0019, ADR-0024, ADR-0026, G-11, G-12
//!
//! Loading is carried over unchanged: a repository built the same way each time, with a merge,
//! branches, a remote branch, an annotated and a lightweight tag, and a stash, loads, lays out,
//! and lists its refs, changed files, and diffs exactly as the reference implementation did,
//! with a clean and with a dirty working tree.

mod fixture_repo;

use ocelli_core::graph::build_graph;
use ocelli_core::model::WORKDIR_SHA;
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use serde_json::Value;
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> (Scratch, PathBuf) {
    let root = std::env::temp_dir().join(format!("ocelli-load-reference-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    (Scratch(root), dir)
}

fn expected(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn a_fixed_repository_loads_as_the_reference_did() {
    let (_guard, dir) = scratch();
    fixture_repo::build(&dir);
    let reader = GitReader;
    let mut quiet = |_: String| {};

    let commits = reader.load_commits(&dir, &mut quiet).unwrap();
    assert_eq!(
        json(&build_graph(commits)),
        expected("repo-graph-clean.json")
    );

    // The worktree's path is where the scratch directory happens to be, spelled as libgit2
    // resolves it (a real path on macOS, forward slashes on Windows); only its last component
    // is the repository's.
    let mut refs = json(&reader.load_refs(&dir).unwrap());
    for worktree in refs["worktrees"].as_array_mut().unwrap() {
        let path = worktree["path"].as_str().unwrap();
        assert!(
            path.trim_end_matches(['/', '\\']).ends_with("repo"),
            "{path}"
        );
        worktree["path"] = "<repo>/".into();
    }
    assert_eq!(refs, expected("repo-refs.json"));

    let (head, merge) = {
        let r = git2::Repository::open(&dir).unwrap();
        let h = r.head().unwrap().peel_to_commit().unwrap();
        let m = h.parent(0).unwrap();
        (h.id().to_string(), m.id().to_string())
    };
    assert_eq!(
        json(&reader.load_commit_files(&dir, &merge).unwrap()),
        expected("repo-files-merge.json")
    );
    assert_eq!(
        json(&reader.load_file_diff(&dir, &head, "a.txt").unwrap()),
        expected("repo-diff-head.json")
    );

    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    let mut graph = json(&build_graph(reader.load_commits(&dir, &mut quiet).unwrap()));
    for node in graph["nodes"].as_array_mut().unwrap() {
        if node["sha"] == WORKDIR_SHA {
            node["date"] = "<now>".into();
            node["authorDate"] = "<now>".into();
        }
    }
    assert_eq!(graph, expected("repo-graph-dirty.json"));
    assert_eq!(
        json(&reader.load_commit_files(&dir, WORKDIR_SHA).unwrap()),
        expected("repo-files-workdir.json")
    );
    assert_eq!(
        json(&reader.load_file_diff(&dir, WORKDIR_SHA, "a.txt").unwrap()),
        expected("repo-diff-workdir.json")
    );
}
