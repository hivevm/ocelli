//! The shared list of commands (ADR-0037, G-18): each host answers the commands the list gives it
//! and no other, the web interface sends no command the list does not name and listens for no
//! event the hosts do not report, and the server's own events are its alone. The commands are
//! read off the sources of the web interface, the desktop host, and the server's dispatch, and the
//! running server is asked each of them.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod browser;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Tab, Tree, server, writing_on};
use ocelli_commands::{Answered, COMMANDS, SERVER_EVENTS};
use serde_json::json;
use std::path::Path;

/// The names `source` passes as the first argument, a string literal, to a call of `function`,
/// type arguments between: `invoke<Found>('search_history', …)` names `search_history`.
fn called_with(source: &str, function: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find(function) {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + function.len()..];
        // A word of its own: `tauriInvoke` and `.invoke` of another name are not `invoke(`.
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let mut after = rest;
        if after.starts_with('<') {
            let mut depth = 0;
            let Some(end) = after.char_indices().find_map(|(i, c)| {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(i)
            }) else {
                continue;
            };
            after = &after[end + 1..];
        }
        let Some(args) = after.strip_prefix('(') else {
            continue;
        };
        let args = args.trim_start();
        let Some(quote) = args
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"' | '`'))
        else {
            continue;
        };
        if let Some(end) = args[1..].find(quote) {
            names.insert(args[1..1 + end].to_string());
        }
    }
    names
}

/// The commands `tauri::generate_handler!` in the desktop host's `lib.rs` registers.
fn desktop_commands(lib: &str) -> std::collections::BTreeSet<String> {
    let list = lib
        .split_once("generate_handler![")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list)
        .expect("the desktop host registers its commands");
    list.split(',')
        .map(|c| c.trim().rsplit("::").next().unwrap_or_default().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// The commands the remote server's dispatch, `answer` in `socket.rs`, has an arm for: each
/// string literal a line of it starts with before `=>`.
fn server_commands(socket: &str) -> std::collections::BTreeSet<String> {
    let body = socket
        .split_once("async fn answer(")
        .and_then(|(_, rest)| rest.split_once("\n}\n"))
        .map(|(body, _)| body)
        .expect("the server dispatches its commands in `answer`");
    body.lines()
        .filter_map(|line| {
            let (name, rest) = line.trim_start().strip_prefix('"')?.split_once('"')?;
            rest.trim_start()
                .starts_with("=>")
                .then(|| name.to_string())
        })
        .collect()
}

/// The commands of the shared list `by` keeps, by name.
fn listed(
    list: &[(&str, Answered)],
    by: impl Fn(Answered) -> bool,
) -> std::collections::BTreeSet<String> {
    list.iter()
        .filter(|(_, how)| by(*how))
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Where the hosts and the shared list of their commands part: each command the list gives a
/// host that the host does not answer, and each a host answers that the list does not give it,
/// named (ADR-0037). A write the server refuses is one it answers, with its refusal.
fn uncovered(
    list: &[(&str, Answered)],
    desktop: &std::collections::BTreeSet<String>,
    server: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let hosts = [
        ("desktop host", desktop, listed(list, Answered::by_desktop)),
        ("server", server, listed(list, Answered::by_server)),
    ];
    let mut parted = Vec::new();
    for (host, answers, given) in hosts {
        for missing in given.difference(answers) {
            parted.push(format!(
                "the command {missing}, which the {host} does not answer"
            ));
        }
        for extra in answers.difference(&given) {
            parted.push(format!(
                "the command {extra}, which the {host} answers and the list does not give it"
            ));
        }
    }
    parted
}

/// What the web interface asks for that the hosts do not carry: each command it sends that the
/// shared list does not name, and each event it listens for that the shared commands, which both
/// hosts carry, never report (G-18, ADR-0037).
///
/// The check decides existence only: that each host answers the commands the list gives it is
/// decided by `uncovered`, and that each answer is the desktop host's for the same repository by
/// `each_read_the_browser_asks_is_answered_as_the_desktop_host_answers_it`.
fn unanswered(
    list: &[(&str, Answered)],
    sent: &std::collections::BTreeSet<String>,
    listened: &std::collections::BTreeSet<String>,
    reported: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let named = listed(list, |_| true);
    let commands = sent
        .difference(&named)
        .map(|c| format!("the command {c}, which the list does not name"));
    let events = listened
        .difference(reported)
        .map(|e| format!("the event {e}"));
    commands.chain(events).collect()
}

/// Where the server's own events and the list of them part (ADR-0037): each the list names that
/// the server never reports, and each the shared commands report too, which the desktop host
/// would then send.
fn misplaced(
    server_events: &[&str],
    by_server: &std::collections::BTreeSet<String>,
    by_shared: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut misplaced = Vec::new();
    for event in server_events.iter().map(|e| e.to_string()) {
        if !by_server.contains(&event) {
            misplaced.push(format!(
                "the server's event {event}, which it never reports"
            ));
        }
        if by_shared.contains(&event) {
            misplaced.push(format!(
                "the server's event {event}, which the shared commands report"
            ));
        }
    }
    misplaced
}

/// The text of every file of the web interface's sources but its tests, or of the shared
/// commands' sources.
fn sources(dir: &Path, keep: impl Fn(&Path) -> bool + Copy) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            text.push_str(&sources(&path, keep));
        } else if keep(&path) {
            text.push_str(&std::fs::read_to_string(&path).unwrap());
            text.push('\n');
        }
    }
    text
}

fn set(names: &[&str]) -> std::collections::BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// A list of four commands, one of each way the hosts answer.
const FOUR: &[(&str, Answered)] = &[
    ("read_window", Answered::ByBoth),
    ("perform_write", Answered::ByBothAsWrite),
    (
        "browse_repo",
        Answered::ByDesktop {
            counterpart: "list_folders",
        },
    ),
    (
        "list_folders",
        Answered::ByServer {
            in_place_of: "browse_repo",
        },
    ),
];

#[test]
// Verifies: ADR-0037
fn a_command_one_host_answers_and_the_other_or_the_list_does_not_is_named() {
    let desktop = set(&["read_window", "perform_write", "browse_repo"]);
    let server = set(&["read_window", "perform_write", "list_folders"]);
    // Both cover the list: nothing.
    assert!(uncovered(FOUR, &desktop, &server).is_empty());
    // A command added to one host and not the other, nor the list.
    let mut more = desktop.clone();
    more.insert("fresh".into());
    assert_eq!(
        uncovered(FOUR, &more, &server),
        ["the command fresh, which the desktop host answers and the list does not give it"]
    );
    let mut more = server.clone();
    more.insert("fresh".into());
    assert_eq!(
        uncovered(FOUR, &desktop, &more),
        ["the command fresh, which the server answers and the list does not give it"]
    );
    // A command listed for both and dropped by one, a refused write included.
    assert_eq!(
        uncovered(FOUR, &desktop, &set(&["read_window", "list_folders"])),
        ["the command perform_write, which the server does not answer"]
    );
    assert_eq!(
        uncovered(FOUR, &set(&["perform_write", "browse_repo"]), &server),
        ["the command read_window, which the desktop host does not answer"]
    );
    // A host answering the other's command of a pair, or not its own.
    assert_eq!(
        uncovered(
            FOUR,
            &desktop,
            &set(&["read_window", "perform_write", "browse_repo"])
        ),
        [
            "the command list_folders, which the server does not answer",
            "the command browse_repo, which the server answers and the list does not give it",
        ]
    );
}

#[test]
// Verifies: G-18, ADR-0037
fn the_existence_check_names_each_command_and_event_of_the_web_interface_no_host_carries() {
    let listened = set(&["history-grew", "set-status"]);
    let reported = set(&["history-grew", "set-status", "write-progress"]);
    // All listed: nothing.
    let sent = set(&["read_window", "browse_repo", "list_folders"]);
    assert!(unanswered(FOUR, &sent, &listened, &reported).is_empty());
    // A command the list does not name: named.
    let sent = set(&["read_window", "web_only"]);
    assert_eq!(
        unanswered(FOUR, &sent, &listened, &reported),
        ["the command web_only, which the list does not name"]
    );
    // An event none of the shared commands reports: named.
    let listened = set(&["history-grew", "folder-chosen"]);
    assert_eq!(
        unanswered(FOUR, &set(&["read_window"]), &listened, &reported),
        ["the event folder-chosen"]
    );
}

#[test]
// Verifies: ADR-0037
fn a_server_event_the_server_never_reports_or_the_shared_commands_report_is_named() {
    let shared = set(&["history-grew", "write-progress"]);
    assert!(misplaced(&["repo-written"], &set(&["repo-written"]), &shared).is_empty());
    assert_eq!(
        misplaced(&["repo-written"], &set(&["set-status"]), &shared),
        ["the server's event repo-written, which it never reports"]
    );
    assert_eq!(
        misplaced(&["write-progress"], &set(&["write-progress"]), &shared),
        ["the server's event write-progress, which the shared commands report"]
    );
}

#[test]
fn the_calls_are_read_off_the_source_as_the_web_interface_writes_them() {
    let source = r#"
        await invoke<RowWindow>('read_window', { ...range });
        invoke('deselect').then(rereadWindow, report);
        invoke<(number | null)[]>("rows_of", { shas });
        reads.ask(() => invoke<Selected | null>(`selection`), currentLoad);
        invoke: (cmd, args) => tauriInvoke(cmd, args),
        transport.invoke(cmd, args);
        reinvoke('not_this');
        listen<Growth>('history-grew', (e) => onGrowth(e.payload)),
    "#;
    assert_eq!(
        called_with(source, "invoke"),
        set(&["deselect", "read_window", "rows_of", "selection"])
    );
    assert_eq!(called_with(source, "listen"), set(&["history-grew"]));
    let lib = "tauri::generate_handler![\n commands::a,\n commands::b_c,\n ])";
    assert_eq!(desktop_commands(lib), set(&["a", "b_c"]));
    let socket = r#"
fn other() { match x { "not_this" => 1 } }
async fn answer(tab: &Tab, cmd: &str) -> Result<Value, String> {
    match cmd {
        "a" => {
            let x = "not_a_command";
            value(x)
        }
        "b_c" => Err(WRITES_REFUSED.into()),
        _ => Err(format!("Unknown command: {cmd}")),
    }
}
fn after() { match y { "nor_this" => 2 } }
"#;
    assert_eq!(server_commands(socket), set(&["a", "b_c"]));
}

#[tokio::test]
// Verifies: G-18, ADR-0037
async fn both_hosts_answer_the_shared_list_of_commands_and_the_web_interface_sends_no_other() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let web = sources(&repo.join("src-web/src"), |p| {
        let name = p.to_string_lossy();
        (name.ends_with(".ts") || name.ends_with(".svelte")) && !name.ends_with(".test.ts")
    });
    let sent = called_with(&web, "invoke");
    let listened = called_with(&web, "listen");
    let desktop =
        desktop_commands(&std::fs::read_to_string(repo.join("src-tauri/src/lib.rs")).unwrap());
    let socket = std::fs::read_to_string(repo.join("crates/server/src/socket.rs")).unwrap();
    let dispatched = server_commands(&socket);
    let shared = sources(&repo.join("crates/commands/src"), |p| {
        p.extension().is_some_and(|e| e == "rs")
    });
    let by_shared = called_with(&shared, "report");
    let by_server = called_with(&socket, "report");
    assert!(by_server.contains("repo-written"), "{by_server:?}");
    let misplaced = misplaced(SERVER_EVENTS, &by_server, &by_shared);
    assert!(misplaced.is_empty(), "{misplaced:?}");
    // What the web interface may listen for: the shared commands' events, which both hosts
    // send, and the server's own, which it listens for in the desktop host to no harm.
    let mut reported = by_shared.clone();
    reported.extend(SERVER_EVENTS.iter().map(|e| e.to_string()));
    // The scans see what they read: the commands and events the web interface's App uses.
    assert!(
        sent.contains("read_window") && sent.contains("open_repo"),
        "{sent:?}"
    );
    assert!(listened.contains("history-grew"), "{listened:?}");
    assert!(desktop.contains("read_window"), "{desktop:?}");
    assert!(dispatched.contains("read_window"), "{dispatched:?}");

    let parted = uncovered(COMMANDS, &desktop, &dispatched);
    assert!(parted.is_empty(), "the hosts and the list part: {parted:?}");
    let missing = unanswered(COMMANDS, &sent, &listened, &reported);
    assert!(
        missing.is_empty(),
        "the hosts do not carry what the web interface asks for: {missing:?}"
    );

    // The running server answers what its dispatch names and only that: the desktop's command
    // it answers in place of is unknown to it. Started without --allow-writes it refuses a write
    // before reading it; with it, it reads the write, which these commands lack (ADR-0037).
    let tree = Tree::new("parity");
    let refused = Err(ocelli_server::socket::WRITES_REFUSED.to_string());
    for writes in [false, true] {
        let running = if writes {
            writing_on(tree.root()).await
        } else {
            server(&tree).await
        };
        let mut tab = Tab::open(&running).await;
        for (command, how) in COMMANDS {
            let answer = tab.invoke(command, json!({})).await;
            let unknown = Err(format!("Unknown command: {command}"));
            match how {
                Answered::ByBothAsWrite if writes => {
                    assert!(
                        answer
                            .as_ref()
                            .is_err_and(|e| e.starts_with("Invalid arguments")),
                        "{command}: {answer:?}"
                    )
                }
                Answered::ByBothAsWrite => assert_eq!(answer, refused, "{command}"),
                Answered::ByDesktop { .. } => assert_eq!(answer, unknown, "{command}"),
                Answered::ByBoth | Answered::ByServer { .. } => {
                    assert_ne!(answer, unknown, "{command}")
                }
            }
        }
    }
}
