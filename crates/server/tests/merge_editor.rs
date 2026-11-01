//! The merge editor through the remote server (ADR-0035, ADR-0031): a tab reads a conflicted
//! file's text and blocks and saves its result over its connection, as the shared commands answer
//! them; a server without `--allow-writes` reads but refuses the save, one confined to its root
//! opens nothing outside it, and a save waits for the repository's turn and checks the file again
//! after the wait.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
// The twin is copied with `cp`, and a link is made, as on unix alone.
#![cfg(unix)]

mod browser;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Pair, Tab, hold_turn, open_and_load, perform, start_on, until_waiting, writing_on};
use ocelli_core::port::{Conflict, Write};
use ocelli_server::Limits;
use serde_json::{Value, json};
use std::path::Path;
use support::{Scratch, git, git_may_fail, snapshot, write_file};

/// A merge stopped in `root` on `dir/f` and `dir/g`, each written `ours` on `main` and `theirs`
/// on `side` from a common `base`, in the `merge` style.
fn stopped_in(root: &Path, dir: &str) {
    std::fs::create_dir_all(root.join(dir)).unwrap();
    git(root, &["init", "--quiet", "-b", "main"]);
    git(root, &["config", "user.name", "Ada"]);
    git(root, &["config", "user.email", "ada@example.invalid"]);
    let both = |content: &str| {
        for f in ["f", "g"] {
            write_file(root, &format!("{dir}/{f}"), content);
        }
        git(root, &["add", "."]);
    };
    both("a\nbase\nz\n");
    git(root, &["commit", "--quiet", "-m", "base"]);
    git(root, &["switch", "--quiet", "-c", "side"]);
    both("a\ntheirs\nz\n");
    git(root, &["commit", "--quiet", "-m", "side"]);
    git(root, &["switch", "--quiet", "main"]);
    both("a\nours\nz\n");
    git(root, &["commit", "--quiet", "-m", "main"]);
    assert!(!git_may_fail(root, &["merge", "side"]));
}

/// The conflict on `path` in `dir` as the web interface shows it, read off `git`.
fn conflict(dir: &Path, path: &str) -> Conflict {
    let mut stages = [None, None, None];
    for line in git(dir, &["ls-files", "-u", "--", path]).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let stage: usize = fields[2].parse().unwrap();
        stages[stage - 1] = Some(fields[1].to_string());
    }
    let [ancestor, ours, theirs] = stages;
    Conflict {
        path: path.into(),
        ancestor,
        ours,
        theirs,
    }
}

/// The save of `result` for `path` in `dir`, opened on the file as it is there now.
fn save(dir: &Path, path: &str, result: &str) -> Write {
    Write::SaveMerge {
        path: path.into(),
        conflict: conflict(dir, path),
        opened: git(dir, &["hash-object", path]),
        result: result.into(),
    }
}

/// The conflicted file `path` as a tab reads it for the merge editor.
async fn read(tab: &mut Tab, path: &str) -> Result<Value, String> {
    let answer = tab
        .invoke("get_merge_file", json!({ "path": path }))
        .await?;
    Ok(serde_json::from_str(answer.as_str().unwrap()).unwrap())
}

/// The paths the stopped operation's banner shows conflicted, from the refs a load sends.
fn conflicted(refs: &Value) -> Vec<String> {
    let conflicts = refs["refs"]["inProgress"]["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("no operation in progress in {refs}"));
    conflicts
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
// Verifies: ADR-0035, ADR-0031, G-15, G-18
async fn with_allow_writes_a_result_saved_in_a_tab_resolves_the_file_as_the_desktop_does() {
    let mut pair = Pair::new("merge-editor", |root| stopped_in(root, "d")).await;
    let file = read(&mut pair.tab, "d/f").await.unwrap();
    assert_eq!(
        file["text"],
        std::fs::read_to_string(pair.root.join("d/f")).unwrap()
    );
    assert_eq!(file["blob"], git(&pair.root, &["hash-object", "d/f"]));
    assert_eq!(file["blocks"][0]["ours"], "ours\n");
    assert_eq!(file["blocks"][0]["theirs"], "theirs\n");
    assert_eq!(
        pair.alike(|d| save(d, "d/f", "a\nours\ntheirs\nz\n"), false)
            .await,
        Ok(Value::Null)
    );
    assert_eq!(
        std::fs::read_to_string(pair.root.join("d/f")).unwrap(),
        "a\nours\ntheirs\nz\n"
    );
    // Loading again as the web interface does after a write, the banner shows what is left.
    let refs = open_and_load(&mut pair.tab, ".").await;
    assert_eq!(conflicted(&refs), ["d/g"]);
}

#[tokio::test]
// Verifies: ADR-0035, ADR-0031, Q-5
async fn without_allow_writes_a_tab_reads_the_file_but_its_save_is_refused_and_nothing_changes() {
    let scratch = Scratch::new("merge-editor-reads");
    let root = scratch.0.join("root");
    stopped_in(&root, "d");
    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    assert_eq!(
        read(&mut tab, "d/f").await.unwrap()["blocks"][0]["ours"],
        "ours\n"
    );
    let before = snapshot(&root);
    // The reason is the server's own.
    assert_eq!(
        perform(&mut tab, &save(&root, "d/f", "a\nz\n")).await,
        Err(ocelli_server::socket::WRITES_REFUSED.to_string())
    );
    assert_eq!(snapshot(&root), before);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0031
async fn a_conflicted_file_below_a_link_that_leads_outside_the_root_is_neither_read_nor_saved() {
    let scratch = Scratch::new("merge-editor-outside");
    let root = scratch.0.join("root");
    stopped_in(&root, "d");
    // The directory the conflict lies in becomes a link to a copy of it outside the root. The
    // git adapter refuses a path below a link wherever it leads, before the server root's
    // confinement could; a path that leaves the root by name is refused as no repository path.
    let outside = scratch.0.join("outside");
    std::fs::rename(root.join("d"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("d")).unwrap();
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = (snapshot(&root), std::fs::read(outside.join("f")).unwrap());
    for path in ["d/f", "../outside/f"] {
        assert_eq!(
            read(&mut tab, path).await,
            Err(ocelli_server::socket::scrub::READ_FAILED.to_string()),
            "{path}"
        );
    }
    let saved = Write::SaveMerge {
        path: "d/f".into(),
        conflict: conflict(&root, "d/f"),
        opened: git(&outside, &["hash-object", "--no-filters", "f"]),
        result: "a\nz\n".into(),
    };
    assert_eq!(
        perform(&mut tab, &saved).await,
        Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string())
    );
    assert_eq!(
        (snapshot(&root), std::fs::read(outside.join("f")).unwrap()),
        before
    );
}

#[tokio::test]
// Verifies: ADR-0035, ADR-0031
async fn a_save_waits_for_the_repository_s_turn_and_checks_the_file_again_after_the_wait() {
    let scratch = Scratch::new("merge-editor-turn");
    let root = scratch.0.join("root");
    stopped_in(&root, "d");
    let running = writing_on(root.clone()).await;
    let common = ocelli_git::refs::common_dir(&root).unwrap();
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let mut other = Tab::open(&running).await;
    open_and_load(&mut other, ".").await;
    // While another write holds the repository's turn, taken here as a tab's write takes it,
    // the file changes: the save sent before waits, and finds the file no longer the one it was
    // opened on.
    let held = hold_turn(&running, &common).await;
    let stale = save(&root, "d/f", "a\nz\n");
    let sent = tab
        .send(
            "perform_write",
            json!({ "write": stale, "confirmed": false }),
        )
        .await;
    until_waiting(&running, &common, 1).await;
    write_file(&root, "d/f", "a\nedited meanwhile\nz\n");
    let before = snapshot(&root);
    drop(held);
    assert_eq!(
        tab.answer(sent).await,
        Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string())
    );
    assert_eq!(snapshot(&root), before);

    // A save of g, its file unchanged while it waits, is made once the turn is free.
    let held = hold_turn(&running, &common).await;
    let fresh = save(&root, "d/g", "a\nboth\nz\n");
    let sent = tab
        .send(
            "perform_write",
            json!({ "write": fresh, "confirmed": false }),
        )
        .await;
    until_waiting(&running, &common, 1).await;
    assert_eq!(
        snapshot(&root),
        before,
        "the save ran while the turn was held"
    );
    drop(held);
    assert_eq!(tab.answer(sent).await, Ok(Value::Null));
    assert_eq!(
        std::fs::read_to_string(root.join("d/g")).unwrap(),
        "a\nboth\nz\n"
    );
    assert_eq!(conflicted(&open_and_load(&mut other, ".").await), ["d/f"]);
}
