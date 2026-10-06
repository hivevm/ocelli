//! Verifies: ADR-0032, G-3
//!
//! The main line's tip, on repositories made with git: the local branch `origin/HEAD` names, else
//! `main`, else `master`, else the checked-out commit; and a load lays the tip's first-parent chain
//! out in lane 0 alone.

use ocelli_core::load::{self, Event, LoadSession};
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ocelli-main-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs git in `dir` with a fixed identity, the commit dates counted up by `GIT_COMMITTER_DATE`
/// as given, and returns its standard output.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// An empty commit on the checked-out branch at `time`, its SHA.
fn commit(dir: &Path, message: &str, time: u64) -> String {
    let date = format!("{time} +0000");
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
        .args(["commit", "--quiet", "--allow-empty", "-m", message])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .unwrap();
    assert!(out.status.success());
    git(dir, &["rev-parse", "HEAD"])
}

/// A repository with one commit on each of `branches`, each on the first, `checkout` checked out.
fn repository(dir: &Path, branches: &[&str], checkout: &str) -> Vec<String> {
    git(dir, &["init", "--quiet", "-b", branches[0]]);
    let mut tips = vec![commit(dir, "root", 1_700_000_000)];
    for (i, branch) in branches.iter().enumerate().skip(1) {
        git(dir, &["switch", "--quiet", "-c", branch, branches[0]]);
        tips.push(commit(dir, branch, 1_700_000_000 + i as u64));
    }
    git(dir, &["switch", "--quiet", checkout]);
    tips
}

fn tip(dir: &Path) -> Option<String> {
    GitReader.load_main_tip(dir).unwrap()
}

#[test]
fn the_branch_origin_head_names_comes_first() {
    let scratch = Scratch::new("origin");
    let tips = repository(&scratch.0, &["main", "trunk", "master"], "master");
    git(
        &scratch.0,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    assert_eq!(tip(&scratch.0), Some(tips[1].clone()));
}

#[test]
fn main_comes_before_master() {
    let scratch = Scratch::new("main");
    let tips = repository(&scratch.0, &["master", "main"], "master");
    assert_eq!(tip(&scratch.0), Some(tips[1].clone()));
}

#[test]
fn master_comes_before_the_checked_out_commit() {
    let scratch = Scratch::new("master");
    let tips = repository(&scratch.0, &["master", "dev"], "dev");
    assert_eq!(tip(&scratch.0), Some(tips[0].clone()));
}

#[test]
fn without_a_main_branch_the_checked_out_commit_stands_in() {
    let scratch = Scratch::new("head");
    let tips = repository(&scratch.0, &["trunk", "dev"], "dev");
    assert_eq!(tip(&scratch.0), Some(tips[1].clone()));
}

#[test]
fn a_remote_branch_alone_does_not_make_a_main_line() {
    let scratch = Scratch::new("remote");
    let tips = repository(&scratch.0, &["trunk", "dev"], "dev");
    git(
        &scratch.0,
        &["update-ref", "refs/remotes/origin/main", &tips[0]],
    );
    assert_eq!(
        tip(&scratch.0),
        Some(tips[1].clone()),
        "the checked-out commit"
    );
}

#[test]
// Verifies: ADR-0032, ADR-0015, G-3
fn a_load_lays_the_main_line_out_in_the_leftmost_lane_alone() {
    let scratch = Scratch::new("load");
    let dir = &scratch.0;
    git(dir, &["init", "--quiet", "-b", "main"]);
    let mut main = vec![commit(dir, "m0", 1_700_000_000)];
    // A feature branch with newer commits than main's tip, merged into main once.
    git(dir, &["switch", "--quiet", "-c", "feature"]);
    let f1 = commit(dir, "f1", 1_700_000_010);
    git(dir, &["switch", "--quiet", "main"]);
    main.push(commit(dir, "m1", 1_700_000_020));
    let merged = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
        .args(["merge", "--quiet", "--no-ff", "-m", "merge", &f1])
        .env("GIT_COMMITTER_DATE", "1700000030 +0000")
        .env("GIT_AUTHOR_DATE", "1700000030 +0000")
        .output()
        .unwrap();
    assert!(merged.status.success());
    main.push(git(dir, &["rev-parse", "HEAD"]));
    git(dir, &["switch", "--quiet", "feature"]);
    let f2 = commit(dir, "f2", 1_700_000_040);

    let mut session = LoadSession::new();
    load::load(&GitReader, dir, &mut |event| {
        if let Event::Chunk(chunk) = event {
            session.append(chunk);
        }
        true
    });
    let history = session.history();
    let lane = |sha: &str| history.node(history.row_of(sha).unwrap()).col;
    for sha in &main {
        assert_eq!(lane(sha), 0, "{sha} on the main line");
    }
    assert!(lane(&f1) > 0 && lane(&f2) > 0, "nothing else in lane 0");
    assert_eq!(
        history.node(0).sha,
        f2,
        "the newest commit, above main's tip"
    );
}
