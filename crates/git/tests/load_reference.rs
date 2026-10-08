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

/// Takes the stash nodes' places out of a graph, which came after the reference implementation
/// (G-7), and returns them with each node's label.
fn stash_places(graph: &mut Value) -> Vec<(String, Value)> {
    let mut places = Vec::new();
    for node in graph["nodes"].as_array_mut().unwrap() {
        if let Some(place) = node.as_object_mut().unwrap().remove("stash") {
            places.push((node["refs"][0].as_str().unwrap().to_string(), place));
        }
    }
    places
}

#[test]
fn a_fixed_repository_loads_as_the_reference_did() {
    let (_guard, dir) = scratch();
    fixture_repo::build(&dir);
    let reader = GitReader;
    let mut quiet = |_: String| {};

    let commits = reader.load_first_chunk(&dir, &mut quiet).unwrap();
    let mut graph = json(&build_graph(commits));
    assert_eq!(
        stash_places(&mut graph),
        [("📦 stash@{0}".to_string(), Value::from(0))]
    );
    assert_eq!(graph, expected("repo-graph-clean.json"));

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
    // Whether a tag is annotated came after the reference implementation (G-7).
    let annotated: Vec<_> = refs["tags"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|tag| tag.as_object_mut().unwrap().remove("annotated").unwrap())
        .collect();
    assert_eq!(annotated, [false, true], "light, v1.0");
    // Whether a branch is merged came after it too (G-4).
    let merged: Vec<_> = refs["locals"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|branch| branch.as_object_mut().unwrap().remove("merged").unwrap())
        .collect();
    assert_eq!(merged, [true, false, true], "feature, hotfix, main");
    // So did the remote a fetch reaches without one named (G-4): none, since the fixture's
    // remote branch was written without a remote configured.
    let default_remote = refs
        .as_object_mut()
        .unwrap()
        .remove("defaultRemote")
        .unwrap();
    assert_eq!(default_remote, serde_json::Value::Null);
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
    let mut graph = json(&build_graph(
        reader.load_first_chunk(&dir, &mut quiet).unwrap(),
    ));
    assert_eq!(stash_places(&mut graph).len(), 1);
    for node in graph["nodes"].as_array_mut().unwrap() {
        if node["sha"] == WORKDIR_SHA {
            node["date"] = "<now>".into();
            node["authorDate"] = "<now>".into();
        }
    }
    assert_eq!(graph, expected("repo-graph-dirty.json"));
    // How much of each change is staged, and the content shown, came after the reference
    // implementation (G-4).
    let mut files = json(&reader.load_commit_files(&dir, WORKDIR_SHA).unwrap());
    let staged: Vec<_> = files
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|f| {
            let f = f.as_object_mut().unwrap();
            let shown = f.remove("shown").unwrap();
            assert!(
                shown["workdir"].is_string(),
                "both files are in the working directory"
            );
            f.remove("staged").unwrap()
        })
        .collect();
    assert!(
        staged.iter().all(|s| s == "no"),
        "nothing is staged: {staged:?}"
    );
    assert_eq!(files, expected("repo-files-workdir.json"));
    assert_eq!(
        json(&reader.load_file_diff(&dir, WORKDIR_SHA, "a.txt").unwrap()),
        expected("repo-diff-workdir.json")
    );
}
