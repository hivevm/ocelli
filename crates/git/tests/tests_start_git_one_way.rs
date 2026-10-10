//! Verifies: Q-4
//!
//! Every test starts `git` through the one helper that runs none of the programs a repository
//! names, so that a marker a Q-4 test finds was left by Ocelli and not by the test's own `git`.
//! The one exception is `git_running_its_programs`, which only the control of a Q-4 test starts,
//! to show that the programs it names would run: the files that may call it are named in
//! `CONTROLS`.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

// Only the unix test of the programs starts git through it; elsewhere the check reads it alone.
#[cfg_attr(not(unix), allow(dead_code))]
mod git_command;
#[cfg(unix)]
mod programs;

#[cfg(unix)]
use git_command::git_command;
use git_command::other_ways;
use std::path::{Path, PathBuf};

/// Every Rust file under the tests of the workspace's crates.
fn test_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            test_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Every Rust file under the tests of the workspace's crates, and the folder of the crates.
fn every_test_file() -> (PathBuf, Vec<PathBuf>) {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut files = Vec::new();
    for krate in std::fs::read_dir(&crates).unwrap() {
        let tests = krate.unwrap().path().join("tests");
        if tests.is_dir() {
            test_files(&tests, &mut files);
        }
    }
    assert!(files.len() > 5, "the tests were found: {files:?}");
    (crates, files)
}

/// The files, below the workspace's crates, whose tests hold the control of a Q-4 test: the only
/// ones that start `git` with the repository's programs on.
const CONTROLS: &[&str] = &["server/tests/allow_writes.rs"];

/// The name of the helper that starts `git` with the repository's programs on.
const RUNNING_ITS_PROGRAMS: &str = "git_running_its_programs";

#[test]
fn only_the_controls_of_q4_start_git_with_the_repositorys_programs_on() {
    let (crates, files) = every_test_file();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/git_command/mod.rs");
    let this = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tests_start_git_one_way.rs");
    let controls: Vec<PathBuf> = CONTROLS.iter().map(|c| crates.join(c)).collect();
    let calls = |f: &Path| {
        std::fs::read_to_string(f)
            .unwrap()
            .contains(RUNNING_ITS_PROGRAMS)
    };
    let found: Vec<&PathBuf> = files
        .iter()
        .filter(|f| **f != helper && **f != this && !controls.contains(f))
        .filter(|f| calls(f))
        .collect();
    assert!(
        found.is_empty(),
        "only the controls of Q-4 start git with the repository's programs on: {found:?}"
    );
    // The check sees what it allows: each control starts it.
    for control in &controls {
        assert!(calls(control), "{} starts no control", control.display());
    }
}

#[test]
fn no_test_starts_git_another_way() {
    let (_, files) = every_test_file();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/git_command/mod.rs");
    let found: Vec<String> = files
        .iter()
        .filter(|f| **f != helper)
        .flat_map(|f| {
            let text = std::fs::read_to_string(f).unwrap();
            other_ways(&text)
                .into_iter()
                .map(move |line| format!("{}:{line}", f.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "start git through git_command instead: {found:?}"
    );
}

#[test]
fn the_check_names_each_line_that_starts_git_another_way() {
    let starts = concat!("Command::new(", "\"git\")");
    let text =
        format!("fn a() {{}}\nlet x = {starts}\n    .arg(\"status\");\nstd::process::{starts};\n");
    assert_eq!(other_ways(&text), vec![2, 4]);
    assert_eq!(
        other_ways("let x = git_command(dir);\nCommand::new(\"cp\");\n"),
        vec![]
    );
    let spaced = concat!("Command :: new ( ", "\"/usr/bin/git\" )");
    let held = concat!("const GIT: &str = ", "\"git\";");
    let text = format!("{spaced}\n{held}\nlet n = \"gitx\";\n");
    assert_eq!(other_ways(&text), vec![1, 2]);
}

#[cfg(unix)]
#[test]
fn git_through_the_helper_runs_none_of_the_repositorys_programs() {
    let root = std::env::temp_dir().join(format!("ocelli-one-way-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = git_command(&repo)
            .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "--quiet", "-b", "main"]);
    std::fs::write(repo.join("f"), "one\n").unwrap();
    git(&["add", "f"]);
    git(&["commit", "--quiet", "-m", "one"]);
    let markers = programs::set_everywhere(&repo, &root.join("tools"));
    std::fs::write(repo.join("f"), "two\n").unwrap();
    for args in [
        &["status", "--porcelain", "--ignored"][..],
        &["ls-files", "--stage"],
        &["diff", "--no-ext-diff"],
        &["diff", "--stat"],
        &["add", "f"],
        &["commit", "--quiet", "-m", "two"],
        &["log", "-p", "-1"],
    ] {
        git(args);
    }
    assert_eq!(markers.ran(), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&root);
}
