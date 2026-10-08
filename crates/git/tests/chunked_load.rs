//! Verifies: ADR-0016
//!
//! The history loads in chunks: repositories of several chunks, made with git, load chunk by chunk
//! in commit-date order, a child dated before its parent after it, together the whole history, and
//! a load stops when it is told to. The chunks lay out into one commit graph.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod git_command;
use git_command::git_command;
use ocelli_core::graph::build_graph;
use ocelli_core::history::History;
use ocelli_core::model::{CHUNK_COMMITS, CommitData, CommitKind};
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ocelli-chunks-{name}-{}", std::process::id()));
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A repository of `commits` commits on a main line with a branch of three merged back every
/// twenty, written by fast-import at fixed times, with a work tree checked out.
fn repository(dir: &Path, commits: usize) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet", "-b", "main"]);
    let mut stream = String::new();
    let (mut mark, mut main) = (0usize, 0usize);
    let commit = |stream: &mut String, branch: &str, mark: usize, parents: &[usize]| {
        stream.push_str(&format!(
            "commit refs/heads/{branch}\nmark :{mark}\ncommitter F <f@x> {} +0000\ndata 1\nc\n",
            1_700_000_000 + mark
        ));
        if let Some(first) = parents.first() {
            stream.push_str(&format!("from :{first}\n"));
        }
        for other in parents.iter().skip(1) {
            stream.push_str(&format!("merge :{other}\n"));
        }
        stream.push('\n');
    };
    while mark < commits {
        if mark > 0 && mark % 20 == 0 && mark + 4 <= commits {
            let base = main;
            for j in 0..3 {
                mark += 1;
                commit(
                    &mut stream,
                    "topic",
                    mark,
                    &[if j == 0 { base } else { mark - 1 }],
                );
            }
            mark += 1;
            commit(&mut stream, "main", mark, &[main, mark - 1]);
        } else {
            mark += 1;
            let parents: &[usize] = if main == 0 { &[] } else { &[main] };
            commit(&mut stream, "main", mark, parents);
        }
        main = mark;
    }
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
    git(dir, &["checkout", "--quiet", "--force", "main"]);
}

fn chunks(dir: &Path) -> Vec<Vec<CommitData>> {
    let mut all = Vec::new();
    GitReader
        .load_chunks(dir, &mut |_| {}, &mut |chunk| {
            all.push(chunk);
            true
        })
        .unwrap();
    all
}

fn shas(commits: &[CommitData]) -> Vec<&str> {
    commits.iter().map(|c| c.sha.as_str()).collect()
}

#[test]
// Verifies: ADR-0016
fn a_history_of_several_chunks_loads_whole_in_commit_date_order() {
    let scratch = Scratch::new("whole");
    let dir = scratch.0.join("repo");
    repository(&dir, 7_100);
    let chunks = chunks(&dir);

    let sizes: Vec<usize> = chunks.iter().map(Vec::len).collect();
    assert_eq!(
        sizes,
        [CHUNK_COMMITS, CHUNK_COMMITS, 7_100 - 2 * CHUNK_COMMITS]
    );
    // The newest CHUNK_COMMITS of the history, as git lists them by date.
    let newest = git(
        &dir,
        &["rev-list", "--all", &format!("--max-count={CHUNK_COMMITS}")],
    );
    let newest: HashSet<&str> = newest.lines().collect();
    let first: HashSet<&str> = shas(&chunks[0]).into_iter().collect();
    assert_eq!(first, newest, "the first chunk holds the newest commits");

    let all: Vec<&CommitData> = chunks.iter().flatten().collect();
    let distinct: HashSet<&str> = all.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(distinct.len(), 7_100, "every commit once");
    let count = git(&dir, &["rev-list", "--count", "--all"]);
    assert_eq!(count.trim(), "7100", "the whole history");

    // Commit-date order, newest first; with no date running backwards, no commit comes before one
    // of its children either.
    assert!(
        all.windows(2).all(|w| w[0].commit_time >= w[1].commit_time),
        "commit-date order"
    );
    let position: HashMap<&str, usize> = all
        .iter()
        .enumerate()
        .map(|(i, c)| (c.sha.as_str(), i))
        .collect();
    for (i, c) in all.iter().enumerate() {
        for p in &c.parent_shas {
            assert!(position[p.as_str()] > i, "{} before its child {}", p, c.sha);
        }
    }
}

#[test]
// Verifies: ADR-0024
fn the_first_chunk_moves_walked_commits_on_to_make_room_for_the_working_directory() {
    let scratch = Scratch::new("dirty");
    let dir = scratch.0.join("repo");
    repository(&dir, 3_010);
    std::fs::write(dir.join("dirty.txt"), "uncommitted").unwrap();
    let chunks = chunks(&dir);

    assert_eq!(chunks[0].len(), CHUNK_COMMITS);
    assert_eq!(chunks[0].last().unwrap().kind, CommitKind::Working);
    let walked: usize = chunks
        .iter()
        .flatten()
        .filter(|c| c.kind == CommitKind::Commit)
        .count();
    assert_eq!(
        walked, 3_010,
        "no walked commit is lost to the working-directory node"
    );
    assert_eq!(
        chunks[1].len(),
        11,
        "the moved commit opens the second chunk"
    );
}

#[test]
fn a_load_stops_when_told_to() {
    let scratch = Scratch::new("stop");
    let dir = scratch.0.join("repo");
    repository(&dir, 7_100);
    let mut handed = 0;
    GitReader
        .load_chunks(&dir, &mut |_| {}, &mut |_| {
            handed += 1;
            false
        })
        .unwrap();
    assert_eq!(handed, 1);
}

#[test]
// Verifies: ADR-0015
fn the_chunks_lay_out_into_one_commit_graph_whose_first_chunk_is_todays() {
    let scratch = Scratch::new("layout");
    let dir = scratch.0.join("repo");
    repository(&dir, 7_100);
    let chunks = chunks(&dir);
    let today = build_graph(chunks[0].clone());

    let mut history = History::new();
    for chunk in chunks {
        history.add_chunk(chunk);
    }
    assert_eq!(history.rows(), 7_100);
    for n in &today.nodes {
        let laid = history.node(history.row_of(&n.sha).unwrap());
        assert_eq!((laid.row, laid.col), (n.row, n.col), "{} moved", n.sha);
    }
    for e in &history.window(0, 7_100).edges {
        assert!(
            e.source_row > e.target_row,
            "a parent above its child: {e:?}"
        );
    }
}

#[test]
// Verifies: ADR-0016
fn a_child_dated_before_its_parent_arrives_after_it_and_every_commit_once() {
    let scratch = Scratch::new("skew");
    let dir = scratch.0.join("repo");
    repository(&dir, 3_100);
    // A branch whose commit sits on the newest commit but carries a date older than any other.
    let head = git(&dir, &["rev-parse", "main"]);
    let head = head.trim();
    let late = git_command(&dir)
        .args([
            "commit-tree",
            &format!("{head}^{{tree}}"),
            "-p",
            head,
            "-m",
            "late",
        ])
        .env("GIT_AUTHOR_NAME", "F")
        .env("GIT_AUTHOR_EMAIL", "f@example.invalid")
        .env("GIT_COMMITTER_NAME", "F")
        .env("GIT_COMMITTER_EMAIL", "f@example.invalid")
        .env("GIT_COMMITTER_DATE", "1000000000 +0000")
        .env("GIT_AUTHOR_DATE", "1000000000 +0000")
        .output()
        .unwrap();
    assert!(late.status.success());
    let late = String::from_utf8(late.stdout).unwrap();
    git(&dir, &["branch", "late", late.trim()]);

    let chunks = chunks(&dir);
    let all: Vec<&CommitData> = chunks.iter().flatten().collect();
    let distinct: HashSet<&str> = all.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(
        (all.len(), distinct.len()),
        (3_101, 3_101),
        "every commit once"
    );
    assert_eq!(
        chunks.last().unwrap().last().unwrap().sha,
        late.trim(),
        "the late child comes last, after its parent in the first chunk"
    );
    assert!(shas(&chunks[0]).contains(&head));
}
