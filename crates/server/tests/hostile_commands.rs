//! Verifies: ADR-0037, ADR-0035, Q-5
//!
//! Every argument of every command comes from the browser, which may send anything JSON holds.
//! git2 unwraps some conversions of a caller's string, and a release build aborts on a panic, so
//! one on any argument would take the remote server down with every tab. Every command of the
//! shared list, reads included, is sent at the WebSocket of a server started with
//! `--allow-writes`, with its arguments, and each of them in turn, set to each hostile value:
//! none may panic, and the server answers each. A path that names the repository beside the
//! server root is refused by every read that takes a path, and nothing there changes.
//!
//! Each command is first sent as made, from what the repository shows, and answered: a hostile
//! value in its place reaches as far. A write is made in the state it needs and each hostile
//! value written to a fresh copy of it, so that what one value wrote never refuses the next.
//!
//! The write guards are layered: `guard::no_nul` refuses a NUL in any string of a write before
//! `guard::valid_ref_name` and `guard::repository_path` see it, and those refuse it again. This
//! test passes with any one of them removed; each has its own refusal test, which fails:
//! `a_nul_in_any_string_of_a_write_is_refused_before_the_repository_is_reached` for `no_nul`,
//! `a_reference_name_with_a_nul_is_no_name_and_reaches_no_panic` and
//! `a_path_is_taken_only_as_plain_names_below_the_working_directory_outside_any_git` in
//! `guard.rs` for the other two. A read guard stands alone: without `repository_path` in
//! `load_file_diff`, a NUL in `get_file_diff`'s `filePath` reaches `DiffOptions::pathspec`, which
//! unwraps it into a C string, and this test names the command, the argument, and the value.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod browser;
#[path = "../../git/tests/reachable/mod.rs"]
mod reachable;
#[path = "../../git/tests/shown/mod.rs"]
mod shown;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Tab, open_and_load, panics_outside_tests, writing_on};
use futures_util::SinkExt;
use ocelli_commands::COMMANDS;
use ocelli_core::model::WORKDIR_SHA;
use ocelli_core::port::Write;
use reachable::{Repositories, State};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use support::{Scratch, git, hostile, shortened, snapshot};
use tokio_tungstenite::tungstenite::Message;

/// How long a command may take to be answered before the server counts as no longer answering.
const ANSWERED_WITHIN: Duration = Duration::from_secs(10);

/// The state of the repository the reads are sent on: one with a change to show.
const READ: State = State::Changed;

/// The folder below the server root the reads are sent on, a copy of [`READ`], so that what the
/// server reads never touches the repository the writes are copied from.
const READ_PATH: &str = "read";

/// The folder below the server root each write is made in.
const RUN: &str = "run";

/// The reads that take a path, and the argument that holds it: each refuses a path that names
/// the repository beside the server root (ADR-0037).
const PATHS: &[(&str, &str)] = &[
    ("open_repo", "/path"),
    ("list_folders", "/path"),
    ("get_file_diff", "/filePath"),
];

/// The arguments a read is sent with as the web interface sends them, from what the repository
/// shows, its newest commit `head`; none for a command the test does not know, which fails it.
/// They are written out here, not read off the server: an argument a command comes to require
/// fails the read as made, but one it comes to take as optional goes unnoticed until it is added.
fn reads(cmd: &str, head: &str) -> Option<Vec<Value>> {
    Some(match cmd {
        "startup_repo" | "browse_repo" | "selection" | "deselect" | "cancel_write" => {
            vec![json!({})]
        }
        "open_repo" => vec![json!({ "path": READ_PATH, "select": head })],
        "list_folders" => vec![json!({ "path": "." })],
        "read_window" => vec![json!({ "top": 0, "rows": 50 })],
        "select_row" => vec![json!({ "row": 0 })],
        "search_history" => vec![json!({ "query": "one" })],
        "rows_of" => vec![json!({ "shas": [head, WORKDIR_SHA] })],
        "select_commit" => vec![json!({ "sha": head }), json!({ "sha": WORKDIR_SHA })],
        "get_file_diff" => vec![
            json!({ "sha": head, "filePath": "k" }),
            json!({ "sha": WORKDIR_SHA, "filePath": "b" }),
        ],
        _ => return None,
    })
}

/// Every argument of `value` but the names of an enum's variants, which a different value would
/// only make unreadable, each as the JSON pointer that leads to it.
fn arguments(value: &Value, at: String, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, field) in map {
                if !matches!(key.as_str(), "kind" | "mode" | "take" | "operation") {
                    arguments(field, format!("{at}/{key}"), found);
                }
            }
        }
        Value::Array(items) => {
            for (n, item) in items.iter().enumerate() {
                arguments(item, format!("{at}/{n}"), found);
            }
        }
        _ => found.push(at),
    }
}

/// The arguments as a whole, then each of them, as JSON pointers.
fn pointers(example: &Value) -> Vec<String> {
    let mut at = vec![String::new()];
    arguments(example, String::new(), &mut at);
    at
}

/// The paths that name the repository `outside`, beside the server root, or what lies in it:
/// from the server root, from a repository below it, and absolute.
fn outside_values(outside: &Path) -> Vec<Value> {
    let mut values = Vec::new();
    for up in ["..", "../.."] {
        for within in ["", "/f", "/.git", "/.git/config"] {
            values.push(json!(format!("{up}/outside{within}")));
        }
    }
    for within in ["", "f", ".git", ".git/config"] {
        values.push(json!(outside.join(within).to_string_lossy()));
    }
    values
}

/// What a browser may send in place of an argument: the hostile strings, the paths of the
/// repository beside the server root, and what is no string at all.
fn values(outside: &Path) -> Vec<Value> {
    let mut values: Vec<Value> = hostile().into_iter().map(Value::String).collect();
    values.extend(outside_values(outside));
    values.extend([
        Value::Null,
        json!(0),
        json!(-1),
        json!(u64::MAX),
        json!(1.5),
        json!(true),
        json!([]),
        json!({}),
    ]);
    values
}

/// `value` as a failure names it, cut short.
fn named(value: &Value) -> String {
    match value {
        Value::String(text) => shortened(text),
        other => shortened(&other.to_string()),
    }
}

/// Reads what arrives in `tab` until `done` holds, within [`ANSWERED_WITHIN`]; why not otherwise.
async fn until_in(tab: &mut Tab, done: impl Fn(&Tab) -> bool) -> Result<(), String> {
    let read = async {
        while !done(tab) {
            let message = tab.next_message().await?;
            tab.keep(message);
        }
        Ok(())
    };
    tokio::time::timeout(ANSWERED_WITHIN, read)
        .await
        .map_err(|_| "not answered".to_string())?
}

/// `cmd` sent in `tab` with `args`: its answer, given with no panic counted while it ran, a load
/// it started included; why there is none otherwise.
async fn sent(tab: &mut Tab, cmd: &str, args: Value) -> Result<Result<Value, String>, String> {
    let before = panics_outside_tests();
    tab.events.clear();
    tab.answers.clear();
    tab.next += 1;
    let id = tab.next;
    let command = json!({ "id": id, "cmd": cmd, "args": args });
    tab.socket
        .send(Message::Text(command.to_string().into()))
        .await
        .map_err(|e| format!("not sent: {e}"))?;
    until_in(tab, |t| t.answered(id)).await?;
    let answer = tab.answer(id).await;
    if cmd == "open_repo" && answer.is_ok() {
        let loaded = |t: &Tab| {
            t.events
                .iter()
                .any(|(e, p)| e == "history-grew" && p["loading"] == false)
        };
        until_in(tab, loaded)
            .await
            .map_err(|why| format!("the load it started: {why}"))?;
    }
    match panics_outside_tests() - before {
        0 => Ok(answer),
        n => Err(format!("{n} panic(s)")),
    }
}

#[test]
fn every_argument_is_a_pointer_of_its_own() {
    let example = json!({ "a": "x", "b": [1, "y"], "c": { "d": true }, "kind": "k" });
    assert_eq!(pointers(&example), ["", "/a", "/b/0", "/b/1", "/c/d"]);
}

#[tokio::test]
async fn every_command_with_each_argument_hostile_panics_nowhere_and_the_server_answers_on() {
    // The count begins before anything of the server runs.
    panics_outside_tests();
    let scratch = Scratch::new("hostile-commands");
    let root = scratch.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let repositories = Repositories::new(&root);
    let read = root.join(READ_PATH);
    repositories.fresh(READ, &read);
    // Beside the server root, the repository the reads are sent on as it was copied, so that a
    // write that reached it would write there as in the repository below the root.
    let outside = scratch.0.join("outside");
    repositories.fresh(READ, &outside);
    let outside_before = snapshot(&outside);
    let head = git(&read, &["rev-parse", "HEAD"]);
    let run = root.join(RUN);
    let values = values(&outside);
    let outside_values = outside_values(&outside);
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    let (mut sent_in_all, mut probes, mut failed) = (0, 0, Vec::new());
    let mut fail = |failure: String| {
        eprintln!("failed: {failure}");
        failed.push(failure);
    };
    for &(cmd, answered) in COMMANDS {
        // What the command is sent with, each in the repository it is sent on, with the state a
        // write needs; the arguments as made are answered, a refusal only where the server
        // answers no command of the name.
        let examples: Vec<(Value, Option<State>)> = match cmd {
            "perform_write" => reachable::examples(&repositories)
                .into_iter()
                .map(|(state, make)| {
                    let write = make(&repositories.of(state));
                    (json!({ "write": write, "confirmed": true }), Some(state))
                })
                .collect(),
            "write_discards" => reachable::examples(&repositories)
                .into_iter()
                .map(|(state, make)| (json!({ "write": make(&repositories.of(state)) }), None))
                .collect(),
            _ => match reads(cmd, &head) {
                Some(reads) => reads.into_iter().map(|args| (args, None)).collect(),
                None => {
                    fail(format!("{cmd}: no hostile arguments are sent for it"));
                    continue;
                }
            },
        };
        for (example, state) in examples {
            let opened = match state {
                Some(state) => {
                    repositories.fresh(state, &run);
                    RUN
                }
                None => READ_PATH,
            };
            open_and_load(&mut tab, opened).await;
            let answer = sent(&mut tab, cmd, example.clone()).await;
            assert!(
                matches!(&answer, Ok(a) if a.is_ok() == answered.by_server()),
                "{cmd} {example} as made: {answer:?}"
            );
            // write_discards reads only its arguments: sent whole, it shows they never reach
            // git, at a cost a write's every argument would multiply.
            let at = if cmd == "write_discards" {
                vec![String::new()]
            } else {
                pointers(&example)
            };
            for pointer in &at {
                for value in &values {
                    let mut args = example.clone();
                    *args.pointer_mut(pointer).unwrap() = value.clone();
                    if let Some(state) = state {
                        repositories.fresh(state, &run);
                    }
                    let answer = sent(&mut tab, cmd, args).await;
                    sent_in_all += 1;
                    let named = format!("{cmd} args{pointer} = {}", named(value));
                    let beside =
                        outside_values.contains(value) && PATHS.contains(&(cmd, pointer.as_str()));
                    match answer {
                        Err(why) => {
                            fail(format!("{named}: {why}"));
                            tab = Tab::open(&running).await;
                            open_and_load(&mut tab, opened).await;
                            continue;
                        }
                        Ok(Ok(_)) if beside => {
                            fail(format!("{named}: answered beside the server root"));
                        }
                        Ok(_) => {}
                    }
                    // A write after an open of the repository beside the root lands where the
                    // tab's repository is: below the root, where it is counted, never beside it.
                    if beside && cmd == "open_repo" {
                        probes += 1;
                        let probe = Write::CreateTag {
                            name: format!("probe-{probes}"),
                            commit: head.clone(),
                            message: None,
                        };
                        let args = json!({ "write": probe, "confirmed": false });
                        match sent(&mut tab, "perform_write", args).await {
                            Ok(Ok(_)) => {}
                            other => fail(format!("{named}, then a write: {other:?}")),
                        }
                    }
                }
            }
        }
    }

    // serde_json refuses a lone surrogate, which a browser's JSON.parse takes: the server closes
    // the tab that sends one before any command reads it, and answers on in another.
    let mut gone = Tab::open(&running).await;
    let before = panics_outside_tests();
    let text = r#"{"id":1,"cmd":"open_repo","args":{"path":"\ud800"}}"#;
    gone.socket.send(Message::Text(text.into())).await.unwrap();
    let closed = tokio::time::timeout(ANSWERED_WITHIN, gone.next_message()).await;
    assert!(matches!(closed, Ok(Err(_))), "{closed:?}");
    assert_eq!(panics_outside_tests(), before);

    assert!(
        failed.is_empty(),
        "{} of {sent_in_all} hostile commands failed: {failed:#?}",
        failed.len()
    );
    // Every probe wrote, below the root, and nothing beside it changed.
    assert!(probes > 0);
    let tags = git(&read, &["tag", "--list", "probe-*"]);
    assert_eq!(tags.lines().count(), probes, "{tags}");
    assert_eq!(snapshot(&outside), outside_before, "beside the server root");
    // No panic outside the tests' own code, while a repository loaded either.
    assert_eq!(panics_outside_tests(), 0);
    // The server answers on, in the tab that sent them all.
    open_and_load(&mut tab, READ_PATH).await;
    let window = tab.invoke("read_window", json!({ "top": 0, "rows": 1 }));
    assert!(window.await.is_ok());
}
