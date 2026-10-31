//! What `--allow-writes` lets through to a repository and what it holds back (ADR-0037): without
//! the switch every write is refused; with it the server answers every write the desktop host
//! answers, a write git refuses changes nothing and names no path, a path that names no file of
//! the working directory is refused without a panic in git2, the writes to one repository take
//! their turn, and none of them runs a program the repository names (Q-4).
//!
//! The panics in git2 are counted for the whole process: a test that asserts on the count reads
//! it at its start, which installs the count, and compares it at its end.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod browser;
#[cfg(unix)]
#[path = "../../git/tests/programs/mod.rs"]
mod programs;
#[path = "../../git/tests/shown/mod.rs"]
mod shown;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{
    Desktop, Tab, Tree, git2_panics, open_and_load, perform, row, server, three_commits, to_value,
    with_changes, writing_on, writing_within,
};
use ocelli_core::port::Write;
use ocelli_server::{Limits, Running};
use serde_json::{Value, json};
use shown::{kinds_of_write, stage_in, unstage_in};
use std::path::Path;
use std::time::Duration;
use support::{git, snapshot};
// The test of Q-4 runs on unix alone, its programs scripts of the shell; `git_command` is the
// module `programs` starts `git` through, as well as its function.
#[cfg(unix)]
use {
    browser::rev,
    support::Scratch,
    support::git_command::{self, git_running_its_programs},
};

#[tokio::test]
// Verifies: ADR-0037, Q-5
async fn without_allow_writes_every_write_is_refused_and_nothing_changes() {
    let tree = Tree::new("write");
    let root = tree.root();
    with_changes(&root);
    git(&root, &["add", "new"]);
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let refused = Err(ocelli_server::socket::WRITES_REFUSED.to_string());
    // The writes a server started with the switch performs, as the web interface sends them,
    // confirmed or not.
    let writes = [
        stage_in(&root, "f"),
        unstage_in(&root, "new"),
        Write::Commit {
            message: "from the browser".into(),
            head: Some(head.clone()),
        },
        Write::CreateTag {
            name: "new".into(),
            commit: head,
            message: None,
        },
    ];
    for write in &writes {
        for confirmed in [false, true] {
            let sent = json!({ "write": write, "confirmed": confirmed });
            assert_eq!(
                tab.invoke("perform_write", sent).await,
                refused,
                "{write:?}"
            );
        }
    }
    // Every kind of write the core takes, whatever it carries.
    let kinds = kinds_of_write();
    assert!(kinds.contains(&"commit".to_string()), "{kinds:?}");
    for kind in kinds {
        let sent = json!({ "write": { "kind": kind }, "confirmed": true });
        assert_eq!(tab.invoke("perform_write", sent).await, refused, "{kind}");
    }
    assert_eq!(snapshot(&root), before);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0039
async fn with_allow_writes_a_write_refused_by_git_changes_nothing_and_names_no_path() {
    let tree = Tree::new("write-refused");
    let root = tree.root();
    with_changes(&root);
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let head = git(&root, &["rev-parse", "HEAD"]);
    // Fetching a remote the repository does not have, staging f as content it does not hold,
    // and committing on a commit not checked out: the git adapter refuses each, in words that
    // reach the browser only as WRITE_FAILED.
    let failed = Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string());
    let fetch = Write::Fetch {
        remote: "origin".into(),
    };
    assert_eq!(perform(&mut tab, &fetch).await, failed);
    let stale = Write::StageFile {
        path: "f".into(),
        shown: Some(head.clone()),
    };
    assert_eq!(perform(&mut tab, &stale).await, failed);
    let elsewhere = Write::Commit {
        message: "from the browser".into(),
        head: Some(git(&root, &["rev-parse", "HEAD~1"])),
    };
    assert_eq!(perform(&mut tab, &elsewhere).await, failed);
    assert_eq!(snapshot(&root), before);
    // As shown, the same file stages.
    assert_eq!(
        perform(&mut tab, &stage_in(&root, "f")).await,
        Ok(Value::Null)
    );
    assert_ne!(snapshot(&root), before);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0039
async fn a_path_that_names_no_file_of_the_working_directory_is_refused_and_the_server_answers_on() {
    let panics = git2_panics();
    let tree = Tree::new("write-paths");
    let root = tree.root();
    with_changes(&root);
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let failed = Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string());
    for path in [
        "",
        ".",
        "./f",
        "../x",
        "/abs",
        "f\0g",
        ".git/config",
        ".GIT/HEAD",
    ] {
        for shown in [None, Some(head.clone())] {
            let stage = Write::StageFile {
                path: path.into(),
                shown: shown.clone(),
            };
            assert_eq!(perform(&mut tab, &stage).await, failed, "{path:?}");
            let unstage = Write::UnstageFile {
                path: path.into(),
                shown,
            };
            assert_eq!(perform(&mut tab, &unstage).await, failed, "{path:?}");
        }
        let diff = json!({ "sha": head, "filePath": path });
        assert_eq!(
            tab.invoke("get_file_diff", diff).await,
            Err(ocelli_server::socket::scrub::READ_FAILED.to_string()),
            "{path:?}"
        );
    }
    // A name with a NUL, which git2 unwraps into a C string, for a tag and a branch.
    for name in ["a\0b", "\0"] {
        let tag = Write::CreateTag {
            name: name.into(),
            commit: head.clone(),
            message: None,
        };
        assert_eq!(perform(&mut tab, &tag).await, failed, "{name:?}");
        let branch = Write::CreateBranch {
            name: name.into(),
            commit: head.clone(),
            switch: false,
        };
        assert_eq!(perform(&mut tab, &branch).await, failed, "{name:?}");
    }
    assert_eq!(snapshot(&root), before);
    assert_eq!(
        git2_panics(),
        panics,
        "git2 panicked on a path or a name the browser sent"
    );
    // The server still answers, and writes a file of the working directory.
    assert_eq!(
        perform(&mut tab, &stage_in(&root, "f")).await,
        Ok(Value::Null)
    );
    assert_ne!(snapshot(&root), before);
}

/// Waits until `n` writes wait for the repository whose common directory is `common`.
async fn until_waiting(running: &Running, common: &Path, n: usize) {
    let since = std::time::Instant::now();
    while running.turns().unwrap().waiting(common) != n {
        assert!(
            since.elapsed() < Duration::from_secs(20),
            "{n} never waited"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
// Verifies: ADR-0037
async fn a_write_past_those_waiting_is_refused_and_a_closed_tab_s_waiting_write_never_runs() {
    let tree = Tree::new("write-waits");
    let root = tree.root();
    with_changes(&root);
    let limits = Limits {
        writes_waiting: 1,
        ..Limits::default()
    };
    let running = writing_within(root.clone(), limits).await;
    let common = ocelli_git::refs::common_dir(&root).unwrap();
    let mut first = Tab::open(&running).await;
    open_and_load(&mut first, ".").await;
    let mut second = Tab::open(&running).await;
    open_and_load(&mut second, ".").await;
    let before = snapshot(&root);
    // A write of another tab holds the repository's turn.
    let held = {
        let (turns, common) = (running.turns().unwrap().clone(), common.clone());
        let never = std::sync::atomic::AtomicBool::new(false);
        tokio::task::spawn_blocking(move || turns.take(common, &never))
            .await
            .unwrap()
            .unwrap()
    };
    let stage = stage_in(&root, "f");
    let write = json!({ "write": stage, "confirmed": false });
    first.send("perform_write", write).await;
    until_waiting(&running, &common, 1).await;
    // As many wait as may: the next is refused at once.
    assert_eq!(
        perform(&mut second, &stage).await,
        Err(ocelli_server::turns::TOO_MANY_WAITING.to_string())
    );
    // The first tab closes while its write waits: the write gives its wait up, the turn still
    // held, so it never ran.
    drop(first);
    until_waiting(&running, &common, 0).await;
    drop(held);
    assert_eq!(snapshot(&root), before);
    // Once the turn is free, a write runs.
    assert_eq!(perform(&mut second, &stage).await, Ok(Value::Null));
    assert_ne!(snapshot(&root), before);
}

/// Expects the server's `repo-written` message, as a tab receives it, to be the fixture the web
/// interface's test reads against its types: where the file differs, it is written anew and the
/// test fails, so that the change to it is committed with the change to the event.
fn the_fixture_holds(message: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repo-written.json");
    let written = serde_json::to_string_pretty(message).unwrap() + "\n";
    if std::fs::read_to_string(&path).unwrap_or_default() != written {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &written).unwrap();
        panic!(
            "{} did not hold what the server sends; it is written anew, commit it",
            path.display()
        );
    }
}

/// Whether `tab` has been told that another tab wrote to its repository, by the time it has its
/// answer to a command sent now: the server sends a tab's events and answers in order.
async fn told_of_a_write(tab: &mut Tab) -> bool {
    tab.invoke("selection", json!({})).await.unwrap();
    tab.events.iter().any(|(event, _)| event == "repo-written")
}

#[tokio::test]
// Verifies: ADR-0037, ADR-draft-merge-editor, G-18
async fn two_tabs_writing_to_one_repository_write_in_turn_and_the_other_tabs_on_it_load_again() {
    let tree = Tree::new("two-tabs-writing");
    let root = tree.root();
    with_changes(&root);
    git(&root, &["add", "f"]);
    // A worktree of the repository and a repository of its own, both below the root.
    git(&root, &["worktree", "add", "--quiet", "wt"]);
    three_commits(&root.join("sub"));
    let running = writing_on(root.clone()).await;
    let common = ocelli_git::refs::common_dir(&root).unwrap();
    let mut tabs = Vec::new();
    for path in [".", ".", "wt", "sub"] {
        let mut tab = Tab::open(&running).await;
        open_and_load(&mut tab, path).await;
        tab.events.clear();
        tabs.push(tab);
    }
    let [mut first, mut second, mut worktree, mut elsewhere]: [Tab; 4] =
        tabs.try_into().ok().unwrap();
    // Both tabs commit on the commit they show, while a write of another holds the turn.
    let head = git(&root, &["rev-parse", "HEAD"]);
    let commit = Write::Commit {
        message: "from the browser".into(),
        head: Some(head.clone()),
    };
    let held = {
        let (turns, common) = (running.turns().unwrap().clone(), common.clone());
        let never = std::sync::atomic::AtomicBool::new(false);
        tokio::task::spawn_blocking(move || turns.take(common, &never))
            .await
            .unwrap()
            .unwrap()
    };
    let sent = json!({ "write": commit, "confirmed": false });
    let one = first.send("perform_write", sent.clone()).await;
    let two = second.send("perform_write", sent).await;
    until_waiting(&running, &common, 2).await;
    drop(held);
    let answers = (first.answer(one).await, second.answer(two).await);
    // One after the other: the second finds HEAD moved, and is refused with nothing changed.
    let failed = Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string());
    let (made, mut winner, mut loser) = match answers {
        (Ok(made), refused) if refused == failed => (made, first, second),
        (refused, Ok(made)) if refused == failed => (made, second, first),
        other => panic!("not one written and one refused: {other:?}"),
    };
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), made.as_str().unwrap());
    assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]), "4");
    // Every other tab on the repository is told, the worktree's included; the tab that wrote,
    // and one on another repository, are not.
    let told = loser.until("repo-written", |_| true).await;
    assert_eq!(told, json!(ocelli_server::socket::WRITTEN_ELSEWHERE));
    the_fixture_holds(&json!({ "event": "repo-written", "payload": told }));
    worktree.until("repo-written", |_| true).await;
    assert!(!told_of_a_write(&mut winner).await, "the tab that wrote");
    assert!(
        !told_of_a_write(&mut elsewhere).await,
        "a tab on another repository"
    );
    // The refused write moved nothing, so it told nobody: the worktree's tab was told once.
    assert!(told_of_a_write(&mut worktree).await);
    let times = worktree
        .events
        .iter()
        .filter(|(event, _)| event == "repo-written")
        .count();
    assert_eq!(times, 1);
    // Loading again as the web interface does, the other tab shows the commit made.
    let refs = open_and_load(&mut loser, ".").await;
    assert_eq!(row(&refs, "locals", "name", "main")["sha"], made);
}

/// One answer to a write of each kind, by kind.
type Answers = [(String, Result<Value, String>)];

/// The writes the server does not answer as the desktop host answers them: each kind whose
/// answer from the server, `server`, is not the one from the desktop host, `desktop`, as the
/// browser would read it (ADR-0037).
fn unperformed(server: &Answers, desktop: &Answers) -> Vec<String> {
    desktop
        .iter()
        .filter_map(|(kind, there)| {
            let here = server.iter().find(|(k, _)| k == kind).map(|(_, a)| a);
            (here != Some(there)).then(|| {
                format!("the write {kind}, which the server answers {here:?} and the desktop host {there:?}")
            })
        })
        .collect()
}

#[test]
// Verifies: ADR-0037
fn a_write_the_desktop_answers_and_the_server_answers_otherwise_or_not_at_all_is_named() {
    let answer = |kind: &str, answer: Result<Value, String>| (kind.to_string(), answer);
    let failed = || Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string());
    let desktop = [
        answer("stageFile", Ok(Value::Null)),
        answer("dropStash", failed()),
        answer("fetch", failed()),
        answer("push", failed()),
    ];
    assert!(unperformed(&desktop, &desktop).is_empty());
    let server = [
        answer("stageFile", Ok(Value::Null)),
        answer("dropStash", failed()),
        answer(
            "fetch",
            Err(ocelli_server::socket::WRITES_REFUSED.to_string()),
        ),
    ];
    assert_eq!(
        unperformed(&server, &desktop),
        [
            format!(
                "the write fetch, which the server answers {:?} and the desktop host {:?}",
                Some(Err::<Value, _>(
                    ocelli_server::socket::WRITES_REFUSED.to_string()
                )),
                failed()
            ),
            format!(
                "the write push, which the server answers None and the desktop host {:?}",
                failed()
            ),
        ]
    );
}

#[tokio::test]
// Verifies: ADR-0037, G-18, Q-5
async fn with_allow_writes_the_server_answers_every_write_the_desktop_answers_and_without_it_none()
{
    // One example of every write the core takes, as the web interface sends it: the core's
    // fixture, which its own test holds to every kind.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/writes.json");
    let examples: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    let kind = |write: &Value| write["kind"].as_str().unwrap().to_string();
    let mut kinds: Vec<String> = examples.iter().map(kind).collect();
    kinds.sort();
    let mut every = kinds_of_write();
    every.sort();
    assert_eq!(kinds, every, "an example of every write");
    // A repository none of the examples applies to, so that each reaches the git adapter and is
    // refused there, or is refused before it, by either host alike.
    let tree = Tree::new("write-parity");
    let root = tree.root();
    let before = snapshot(&root);
    let (desktop, _) = Desktop::open(&root).await;
    let mut there = Vec::new();
    for write in &examples {
        let write: Write = serde_json::from_value(write.clone()).unwrap();
        there.push((kind(&to_value(&write)), desktop.write(&write, true).await));
    }
    assert!(
        there
            .iter()
            .all(|(_, a)| *a != Err(ocelli_server::socket::WRITES_REFUSED.to_string())),
        "the desktop host answers every write"
    );
    for writes in [false, true] {
        let running = if writes {
            writing_on(root.clone()).await
        } else {
            server(&tree).await
        };
        let mut tab = Tab::open(&running).await;
        open_and_load(&mut tab, ".").await;
        let mut here = Vec::new();
        for write in &examples {
            let sent = json!({ "write": write, "confirmed": true });
            here.push((kind(write), tab.invoke("perform_write", sent).await));
        }
        let unperformed = unperformed(&here, &there);
        if writes {
            assert!(unperformed.is_empty(), "{unperformed:?}");
        } else {
            assert_eq!(unperformed.len(), examples.len(), "{unperformed:?}");
            for (kind, answer) in &here {
                assert_eq!(
                    answer,
                    &Err(ocelli_server::socket::WRITES_REFUSED.to_string()),
                    "{kind}"
                );
            }
        }
        assert_eq!(snapshot(&root), before, "with --allow-writes: {writes}");
    }
}

/// A port on this machine that accepts each connection and closes it at once: an SSH client finds
/// no server there.
#[cfg(unix)]
fn closing_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    port
}

/// `f` of five lines, its first and its last as given.
#[cfg(unix)]
fn five_lines(first: &str, last: &str) -> String {
    format!("{first}\n2\n3\n4\n{last}\n")
}

/// The repository the test of Q-4 writes to at `base/root`, which holds three commits, by Ada,
/// whom its configuration names, with every program named and the scripts in `tools`; the markers
/// they leave:
///
/// - `main`: `f` of five lines, its last changed in the working directory, and `new`, untracked
/// - `side`: `f`'s first line changed, so that merging it merges the content of `f`
/// - `origin`: a bare clone at `base/remote.git`, below the server root, its branch `upstream` a
///   commit ahead
/// - `far`: an SSH address at `port` on this machine
#[cfg(unix)]
fn with_programs(base: &Path, port: u16, tools: &Path) -> programs::Markers {
    let root = base.join("root");
    git(&root, &["config", "user.name", "Ada"]);
    git(&root, &["config", "user.email", "ada@example.invalid"]);
    std::fs::write(root.join("f"), five_lines("1", "5")).unwrap();
    git(&root, &["commit", "--quiet", "-am", "five lines"]);
    git(&root, &["switch", "--quiet", "-c", "side"]);
    std::fs::write(root.join("f"), five_lines("side", "5")).unwrap();
    git(&root, &["commit", "--quiet", "-am", "side"]);
    git(&root, &["switch", "--quiet", "main"]);
    std::fs::write(root.join("f"), five_lines("1", "main")).unwrap();
    std::fs::write(root.join("new"), "new\n").unwrap();
    git(base, &["clone", "--quiet", "--bare", "root", "remote.git"]);
    let remote = base.join("remote.git");
    let ahead = git(
        &remote,
        &["commit-tree", "main^{tree}", "-p", "main", "-m", "ahead"],
    );
    git(&remote, &["update-ref", "refs/heads/upstream", &ahead]);
    git(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let far = format!("ssh://127.0.0.1:{port}/x");
    git(&root, &["remote", "add", "far", &far]);
    let markers = programs::set_everywhere(&root, tools);
    // Ocelli refuses what these would have git sign (ADR-0033); the writes here go through.
    let mut config = git2::Repository::open(&root).unwrap().config().unwrap();
    for (key, _) in programs::SWITCHES {
        config.remove(key).unwrap();
    }
    markers
}

/// The programs the writes of the test of Q-4 would run where `git` performed them, each by the
/// marker it leaves. The credential helper is left to the test of Q-6 in `transfers.rs`, over a
/// transport that asks for credentials, which neither of these does.
#[cfg(unix)]
const WOULD_RUN: &[&str] = &[
    // Staging, unstaging, and committing.
    "core.fsmonitor",
    "hook-post-index-change",
    "filter.evil.process",
    "hook-pre-commit",
    "hook-prepare-commit-msg",
    "hook-commit-msg",
    "hook-post-commit",
    // Every write that moves a ref: committing, stashing, merging, fetching, and pushing.
    "hook-reference-transaction",
    // Merging `side`, which merges the content of `f`.
    "merge.evil.driver",
    "hook-pre-merge-commit",
    "hook-post-merge",
    // Fetching and pushing over the local transport.
    "remote.origin.uploadpack",
    "remote.origin.receivepack",
    "hook-pre-push",
    // Fetching over SSH.
    "core.sshCommand",
];

#[cfg(unix)]
#[tokio::test]
// Verifies: ADR-0037, Q-4
async fn the_server_s_writes_run_none_of_the_programs_the_repository_names() {
    let port = closing_port();
    // The control: `git`, on a repository made alike, performing what the server performs below,
    // runs each program the test names.
    let control = Scratch::new("write-programs-control");
    let there = control.0.join("root");
    three_commits(&there);
    let control_tools = Scratch::new("write-programs-control-ran");
    let control_markers = with_programs(&control.0, port, &control_tools.0);
    for args in [
        &["add", "f"][..],
        &["add", "new"],
        &["reset", "--quiet", "--", "new"],
        &["add", "new"],
        &["commit", "--quiet", "-m", "from the browser"],
        &["stash", "push", "--quiet", "--include-untracked"],
        &["merge", "--quiet", "--no-edit", "side"],
        &["stash", "apply", "--quiet"],
        &["fetch", "--quiet", "origin"],
        &["push", "--quiet", "origin", "main"],
        &["fetch", "--quiet", "far"],
    ] {
        if args[0] == "stash" && args[1] == "push" {
            std::fs::write(there.join("new"), "stashed\n").unwrap();
            std::fs::write(there.join("loose"), "untracked\n").unwrap();
        }
        // Whether `git` succeeds is not the control's question: a program of the test's stops
        // some of these, a filter that answers no greeting and an SSH command that connects
        // nowhere, once it ran.
        git_running_its_programs(&there)
            .args(args)
            .output()
            .unwrap();
    }
    let mut ran = control_markers.ran();
    ran.sort();
    let mut would_run: Vec<String> = WOULD_RUN.iter().map(|m| m.to_string()).collect();
    would_run.sort();
    assert_eq!(
        ran, would_run,
        "the programs git runs are those the test names"
    );

    let tree = Tree::new("write-programs");
    let root = tree.root();
    // Outside the server root, so that the scripts and their markers are none of its files.
    let tools = Scratch::new("write-programs-ran");
    let markers = with_programs(&tree.0, port, &tools.0);
    let running = writing_on(tree.0.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, "root").await;
    let head = rev(&root, "HEAD");
    // Each write made from what the repository shows once the one before it ran.
    let writes: [fn(&Path) -> Write; 4] = [
        |root| stage_in(root, "f"),
        |root| stage_in(root, "new"),
        |root| unstage_in(root, "new"),
        |root| stage_in(root, "new"),
    ];
    for write in writes {
        let write = write(&root);
        assert_eq!(
            perform(&mut tab, &write).await,
            Ok(Value::Null),
            "{write:?}"
        );
    }
    let commit = Write::Commit {
        message: "from the browser".into(),
        head: Some(head),
    };
    let made = perform(&mut tab, &commit).await.unwrap();
    assert_eq!(made, json!(rev(&root, "HEAD")), "committed");
    std::fs::write(root.join("new"), "stashed\n").unwrap();
    std::fs::write(root.join("loose"), "untracked\n").unwrap();
    let stash = Write::CreateStash {
        message: None,
        untracked: true,
    };
    assert_eq!(perform(&mut tab, &stash).await, Ok(Value::Null), "stashed");
    assert!(
        !root.join("loose").exists(),
        "the stash took the untracked file"
    );
    let merge = Write::Merge {
        branch: "main".into(),
        tip: rev(&root, "main"),
        commit: rev(&root, "side"),
        name: Some("side".into()),
    };
    let merged = perform(&mut tab, &merge).await.unwrap();
    assert_eq!(merged, json!(rev(&root, "HEAD")), "merged");
    let parents = git(&root, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split(' ').count(), 3, "a merge commit: {parents}");
    assert_eq!(
        std::fs::read_to_string(root.join("f")).unwrap(),
        five_lines("side", "main"),
        "the content of f merged"
    );
    let apply = Write::ApplyStash {
        index: 0,
        commit: rev(&root, "refs/stash"),
        drop: false,
    };
    assert_eq!(perform(&mut tab, &apply).await, Ok(Value::Null), "applied");
    assert_eq!(
        std::fs::read_to_string(root.join("new")).unwrap(),
        "stashed\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("loose")).unwrap(),
        "untracked\n"
    );
    let fetch = |remote: &str| Write::Fetch {
        remote: remote.into(),
    };
    assert_eq!(
        perform(&mut tab, &fetch("origin")).await,
        Ok(Value::Null),
        "fetched"
    );
    assert_eq!(
        rev(&root, "origin/upstream"),
        rev(&tree.0.join("remote.git"), "upstream"),
        "the fetch brought origin's upstream"
    );
    let push = Write::Push {
        branch: "main".into(),
        tip: rev(&root, "main"),
    };
    assert_eq!(perform(&mut tab, &push).await, Ok(Value::Null), "pushed");
    assert_eq!(
        rev(&tree.0.join("remote.git"), "main"),
        rev(&root, "main"),
        "the push moved origin's main"
    );
    // The fetch over SSH reaches the address and fails there, which the server reports as a
    // failed write and the desktop host, on the same repository, in git's words.
    assert_eq!(
        perform(&mut tab, &fetch("far")).await,
        Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string())
    );
    let (desktop, _) = Desktop::open(&root).await;
    let said = desktop.write_saying(&fetch("far"), false).await;
    assert!(
        matches!(&said, Err(ocelli_commands::HostError::Git(why))
            if why.contains("failed to start SSH session")),
        "the fetch over SSH fails at the address, no guard before it: {said:?}"
    );
    let ran = markers.ran();
    assert!(ran.is_empty(), "programs ran: {ran:?}");
}
