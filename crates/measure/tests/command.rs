//! End-to-end tests of `ocelli-measure crossings` on repositories built with git.

#[path = "../../git/tests/support/mod.rs"]
mod support;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::{Scratch, git, git_command};

/// Feeds a fast-import stream into the bare repository at `dir`.
fn fast_import(dir: &Path, stream: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet", "--bare"]);
    let mut child = git_command(dir)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    git(dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
}

/// A fast-import commit on `branch` with mark `mark`, at time `time`, with the given parents.
fn commit(stream: &mut String, branch: &str, mark: usize, time: u64, parents: &[usize]) {
    stream.push_str(&format!(
        "commit refs/heads/{branch}\nmark :{mark}\ncommitter F <f@example.invalid> {time} +0000\ndata 1\nc\n"
    ));
    if let Some(first) = parents.first() {
        stream.push_str(&format!("from :{first}\n"));
    }
    for other in parents.iter().skip(1) {
        stream.push_str(&format!("merge :{other}\n"));
    }
    stream.push('\n');
}

fn measure(repo: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ocelli-measure"))
        .arg("crossings")
        .arg(repo)
        .output()
        .unwrap()
}

/// The value after `key: ` in the command's output.
fn value(out: &Output, key: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))
        .unwrap_or_else(|| panic!("no {key} in {out:?}"))
        .parse()
        .unwrap()
}

fn assert_measured(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const LIMIT: usize = ocelli_core::model::CHUNK_COMMITS;

#[test]
// Verifies: ADR-0018
fn a_stash_and_uncommitted_changes_are_left_out_of_the_comparison() {
    let scratch = Scratch::new("stash");
    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "--quiet", "-b", "main"]);
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        std::fs::write(dir.join("a.txt"), text).unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "--quiet", "-m", &format!("commit {i}")]);
    }
    std::fs::write(dir.join("a.txt"), "stashed").unwrap();
    git(&dir, &["stash", "--quiet"]);
    std::fs::write(dir.join("a.txt"), "dirty").unwrap();

    let out = measure(&dir);
    assert_measured(&out);
    assert_eq!(value(&out, "commits"), 3, "the three real commits only");
}

#[test]
// Verifies: ADR-0016
fn beyond_the_commit_limit_both_sides_see_the_same_newest_commits() {
    let scratch = Scratch::new("limit");
    let dir = scratch.0.join("repo.git");
    let mut stream = String::new();
    let (mut mark, mut time, mut main) = (0, 1_700_000_000, 0);
    // A main line with a branch of three commits merged back every twenty, across the cut.
    while mark < LIMIT + 300 {
        if mark > 0 && mark % 20 == 0 {
            let base = main;
            for j in 0..3 {
                mark += 1;
                time += 1;
                commit(
                    &mut stream,
                    "topic",
                    mark,
                    time,
                    &[if j == 0 { base } else { mark - 1 }],
                );
            }
            mark += 1;
            time += 1;
            commit(&mut stream, "main", mark, time, &[main, mark - 1]);
        } else {
            mark += 1;
            time += 1;
            let parents: &[usize] = if main == 0 { &[] } else { &[main] };
            commit(&mut stream, "main", mark, time, parents);
        }
        main = mark;
    }
    fast_import(&dir, &stream);

    let out = measure(&dir);
    assert_measured(&out);
    assert_eq!(value(&out, "commits"), LIMIT);
}

#[test]
// Verifies: ADR-0016
fn a_merge_whose_second_parent_lies_beyond_the_cut_is_compared_over_the_same_commits() {
    let scratch = Scratch::new("old-branch");
    let dir = scratch.0.join("repo.git");
    let mut stream = String::new();
    let start = 1_700_000_000;
    // A topic branch forked from the first commit at the oldest times, merged at the very top
    // of a main line longer than the limit, so its commits fall beyond the cut.
    commit(&mut stream, "main", 1, start, &[]);
    for t in 2..=4 {
        commit(&mut stream, "topic", t, start + t as u64, &[t - 1]);
    }
    let mut main = 1;
    for m in 5..LIMIT + 200 {
        commit(&mut stream, "main", m, start + 1_000 + m as u64, &[main]);
        main = m;
    }
    let merge = LIMIT + 200;
    commit(&mut stream, "main", merge, start + 10_000, &[main, 4]);
    fast_import(&dir, &stream);

    let out = measure(&dir);
    assert_measured(&out);
    assert_eq!(value(&out, "commits"), LIMIT);
    value(&out, "ocelli crossings");
    value(&out, "git crossings");
}

// The marker programs are shell scripts with an executable bit, which Git for Windows starts
// through its own shell under rules of its own; the isolation is the same code on every platform,
// so Unix decides it.
#[cfg(unix)]
#[test]
// Verifies: Q-4
fn measuring_a_repository_starts_none_of_the_programs_its_configuration_names() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new("config");
    let dir = scratch.0.join("repo");
    let markers = scratch.0.join("markers");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&markers).unwrap();
    git(&dir, &["init", "--quiet", "-b", "main"]);
    git(&dir, &["commit", "--quiet", "--allow-empty", "-m", "first"]);

    // A commit carrying a signature, which `log.showSignature` would hand to `gpg.program`.
    let tree = git(&dir, &["write-tree"]).trim().to_string();
    let parent = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    let object = format!(
        "tree {tree}\nparent {parent}\nauthor F <f@x> 1767225600 +0000\ncommitter F <f@x> 1767225600 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n -----END PGP SIGNATURE-----\n\nsigned\n"
    );
    let mut hash = git_command(&dir)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hash.stdin
        .take()
        .unwrap()
        .write_all(object.as_bytes())
        .unwrap();
    let signed = String::from_utf8(hash.wait_with_output().unwrap().stdout).unwrap();
    git(&dir, &["update-ref", "refs/heads/main", signed.trim()]);

    // Every place a repository can name a program leaves a marker. Of these, git log starts only
    // the signature program by itself; the isolation in the command is what keeps it out, while
    // the filesystem monitor, the pager, and the hooks stay here so a new way in is noticed.
    let script = |name: &str| {
        let path = scratch.0.join(format!("{name}.sh"));
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch '{}/{name}'\ncat\n", markers.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    };
    let hooks = scratch.0.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for hook in [
        "post-checkout",
        "pre-commit",
        "post-commit",
        "reference-transaction",
    ] {
        let path = hooks.join(hook);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch '{}/hook-{hook}'\n", markers.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(&dir, &["config", "log.showSignature", "true"]);
    git(&dir, &["config", "gpg.program", &script("gpg")]);
    git(&dir, &["config", "core.fsmonitor", &script("fsmonitor")]);
    git(&dir, &["config", "core.pager", &script("pager")]);
    git(
        &dir,
        &["config", "core.hooksPath", &hooks.display().to_string()],
    );

    let out = measure(&dir);
    assert_measured(&out);
    let ran: Vec<_> = std::fs::read_dir(&markers)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(ran.is_empty(), "measuring ran {ran:?}");
}

#[test]
// Verifies: Q-2, ADR-0016
fn the_windows_measurement_loads_every_chunk_and_reports_each_wait() {
    let scratch = Scratch::new("windows");
    let repo = scratch.0.join("repo.git");
    let mut stream = String::new();
    for mark in 1..=3100 {
        let parents: Vec<usize> = if mark > 1 { vec![mark - 1] } else { vec![] };
        commit(
            &mut stream,
            "main",
            mark,
            1_700_000_000 + mark as u64,
            &parents,
        );
    }
    fast_import(&repo, &stream);
    let out = Command::new(env!("CARGO_BIN_EXE_ocelli-measure"))
        .args(["windows"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("commits: 3100\n"),
        "both chunks loaded: {text}"
    );
    assert!(
        text.contains("windows: 52\n"),
        "60 rows to a window: {text}"
    );
    for key in [
        "first window ms",
        "load ms",
        "slowest while loading ms",
        "slowest window ms",
        "lineage ms",
        "search ms",
    ] {
        assert!(
            text.lines().any(|l| l
                .strip_prefix(key)
                .and_then(|v| v.strip_prefix(": "))
                .is_some_and(|v| v.parse::<f64>().is_ok())),
            "{key} in {text}"
        );
    }
}

/// The main line the command reports it laid out with.
fn main_line(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("main line: "))
        .unwrap_or_else(|| panic!("no main line in {out:?}"))
        .to_string()
}

#[test]
// Verifies: ADR-0014, Q-1
fn crossings_are_measured_on_the_layout_with_the_main_line() {
    let scratch = Scratch::new("main-line");
    let repo = scratch.0.join("repo.git");
    let mut stream = String::new();
    commit(&mut stream, "main", 1, 1_700_000_001, &[]);
    commit(&mut stream, "main", 2, 1_700_000_002, &[1]);
    fast_import(&repo, &stream);
    let tip = git(&repo, &["rev-parse", "main"]);
    let out = measure(&repo);
    assert_measured(&out);
    assert_eq!(main_line(&out), tip.trim(), "the main branch's tip");

    // A repository whose HEAD names no branch that exists, and with no main branch, has none.
    git(&repo, &["branch", "-q", "-m", "main", "other"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/gone"]);
    let out = measure(&repo);
    assert_measured(&out);
    assert_eq!(main_line(&out), "none");
}
