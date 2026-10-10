//! The remote server tested where the browser meets it: over HTTP and the WebSocket, against a
//! server started in the test on a free port and a repository `git` made.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod browser;
#[path = "../../git/tests/shown/mod.rs"]
mod shown;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::*;
use futures_util::{SinkExt, StreamExt};
use ocelli_commands::{Answered, COMMANDS, Host, SERVER_EVENTS};
use ocelli_core::port::Write;
use ocelli_server::{Limits, Options, Running, start};
use serde_json::{Value, json};
use shown::{kinds_of_write, stage_in, unstage_in};
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Scratch, git, git_command, snapshot};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

#[tokio::test]
// Verifies: ADR-0037, ADR-0038, ADR-0039, G-8
async fn the_printed_url_opens_the_repository_at_the_root_and_its_rows_arrive_as_the_desktop_reads_them()
 {
    let tree = Tree::new("tracer");
    let running = server(&tree).await;
    assert!(running.url.starts_with("http://localhost:"));

    // The token is exchanged for a session cookie and a redirect to the URL without it.
    let answer = get(running.port, &target(&running), &[]).await;
    assert_eq!(answer.status, 303);
    assert_eq!(answer.header("location"), Some("/"));
    let set = answer.header("set-cookie").unwrap();
    for part in ["ocelli-session=", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(set.contains(part), "{part} in {set}");
    }
    let cookie = set.split(';').next().unwrap().to_string();

    // With the cookie the web interface is served, under the headers of ADR-0037 and ADR-0038.
    let page = get(running.port, "/", &[("Cookie", &cookie)]).await;
    if ocelli_server::web::built() {
        assert_eq!(page.status, 200);
        assert!(page.body.contains("<title>Ocelli</title>"), "{}", page.body);
    } else {
        assert_eq!(page.status, 503);
    }
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(
        csp.contains("img-src 'self' https://www.gravatar.com;"),
        "{csp}"
    );
    assert_eq!(page.header("referrer-policy"), Some("no-referrer"));

    // The tab asks for the repository to open at its start, opens it, and reads its rows.
    let origin = format!("http://localhost:{}", running.port);
    let socket = connect(running.port, Some(&cookie), Some(&origin))
        .await
        .unwrap();
    let mut tab = Tab {
        socket,
        events: Vec::new(),
        answers: Vec::new(),
        next: 0,
    };
    assert_eq!(tab.invoke("startup_repo", json!({})).await, Ok(json!(".")));
    assert_eq!(tab.invoke("startup_repo", json!({})).await, Ok(Value::Null));
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "." })).await,
        Ok(Value::Null)
    );
    let grown = tab.until("history-grew", |p| p["loading"] == false).await;
    assert_eq!(grown["rows"], 3);
    let refs = tab.until("load-refs", |_| true).await;
    assert_eq!(refs["refs"]["locals"][0]["name"], "main");
    assert!(
        tab.events
            .contains(&("set-repo-label".to_string(), json!("."))),
        "{:?}",
        tab.events
    );
    let window = tab
        .invoke("read_window", json!({ "top": 0, "rows": 50 }))
        .await
        .unwrap();
    let head = git(&tree.root(), &["rev-parse", "HEAD"]);
    assert_eq!(window["nodes"][0]["sha"], head.as_str());
    assert_eq!(window["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(window, desktop_window(&tree.root(), 0, 50).await);
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn a_request_without_the_token_or_its_cookie_is_refused_and_serves_nothing() {
    let tree = Tree::new("refused");
    let running = server(&tree).await;
    let port = running.port;
    // That the cookie opens what is refused without it.
    let cookie = log_in(&running).await;
    let origin = format!("http://localhost:{port}");
    assert!(connect(port, Some(&cookie), Some(&origin)).await.is_ok());

    for target in ["/", "/index.html", "/assets/index.js", "/ws"] {
        let answer = get(port, target, &[]).await;
        assert_eq!(answer.status, 403, "{target}");
        assert_eq!(
            answer.body,
            "Open the URL ocelli-server printed at its start"
        );
    }
    let wrong = "0".repeat(64);
    let answer = get(port, &format!("/?token={wrong}"), &[]).await;
    assert_eq!(answer.status, 403);
    assert_eq!(answer.header("set-cookie"), None);
    let forged = format!("ocelli-session={wrong}");
    assert_eq!(get(port, "/", &[("Cookie", &forged)]).await.status, 403);
    assert_eq!(connect(port, None, Some(&origin)).await.err(), Some(403));
    assert_eq!(
        connect(port, Some(&forged), Some(&origin)).await.err(),
        Some(403)
    );
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn a_handshake_from_another_origin_or_none_and_a_foreign_host_are_refused() {
    let tree = Tree::new("origin");
    let running = server(&tree).await;
    let port = running.port;
    let cookie = log_in(&running).await;
    assert_eq!(
        connect(port, Some(&cookie), Some("http://evil.example"))
            .await
            .err(),
        Some(403)
    );
    assert_eq!(connect(port, Some(&cookie), None).await.err(), Some(403));
    let foreign = get(
        port,
        "/",
        &[("Cookie", &cookie), ("Origin", "http://evil.example")],
    )
    .await;
    assert_eq!(foreign.status, 403);
    let rebound = get_as(
        port,
        &format!("evil.example:{port}"),
        "/",
        &[("Cookie", &cookie)],
    )
    .await;
    assert_eq!(rebound.status, 403);
    // A tunnel's local port is another port of localhost, and the cookie still opens it.
    let tunnel = get_as(port, "localhost:1", "/", &[("Cookie", &cookie)]).await;
    assert_ne!(tunnel.status, 403);
}

#[tokio::test]
// Verifies: ADR-0039
async fn a_path_outside_the_server_root_is_refused() {
    let tree = Tree::new("outside");
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    let outside = tree.0.join("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, tree.root().join("link")).unwrap();
    let mut paths = vec![
        "../outside".to_string(),
        outside.to_string_lossy().into_owned(),
    ];
    if cfg!(unix) {
        paths.push("link".into());
    }
    for path in paths {
        assert_eq!(
            tab.invoke("open_repo", json!({ "path": path })).await,
            Err(ocelli_server::root::refused(&path))
        );
    }
    assert!(
        !tab.events.iter().any(|(e, _)| e == "set-repo-label"),
        "nothing was opened: {:?}",
        tab.events
    );
    // The same tab opens what lies below the server root.
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "." })).await,
        Ok(Value::Null)
    );
}

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
    let failed = Err(ocelli_server::socket::WRITE_FAILED.to_string());
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

/// How many panics inside git2 this process has had since the count began: the release build
/// aborts on a panic, so one there takes the whole server down with every tab, where a test
/// build only fails the command. The panics are still reported as they were.
fn git2_panics() -> &'static std::sync::atomic::AtomicUsize {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.location().is_some_and(|l| l.file().contains("git2-")) {
                COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            previous(info);
        }));
    });
    &COUNT
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0039
async fn a_path_that_names_no_file_of_the_working_directory_is_refused_and_the_server_answers_on() {
    let panics = git2_panics().load(std::sync::atomic::Ordering::SeqCst);
    let tree = Tree::new("write-paths");
    let root = tree.root();
    with_changes(&root);
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let failed = Err(ocelli_server::socket::WRITE_FAILED.to_string());
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
            Err(ocelli_server::socket::READ_FAILED.to_string()),
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
        git2_panics().load(std::sync::atomic::Ordering::SeqCst),
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
    let common = ocelli_git::confine::common_dir(&root).unwrap();
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
// Verifies: ADR-0037, ADR-0033, G-8
async fn two_tabs_writing_to_one_repository_write_in_turn_and_the_other_tabs_on_it_load_again() {
    let tree = Tree::new("two-tabs-writing");
    let root = tree.root();
    with_changes(&root);
    git(&root, &["add", "f"]);
    // A worktree of the repository and a repository of its own, both below the root.
    git(&root, &["worktree", "add", "--quiet", "wt"]);
    three_commits(&root.join("sub"));
    let running = writing_on(root.clone()).await;
    let common = ocelli_git::confine::common_dir(&root).unwrap();
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
    let failed = Err(ocelli_server::socket::WRITE_FAILED.to_string());
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

#[cfg(unix)]
#[path = "../../git/tests/programs/mod.rs"]
mod programs;

#[cfg(unix)]
#[tokio::test]
// Verifies: ADR-0037, Q-4
async fn the_server_s_writes_run_none_of_the_programs_the_repository_names() {
    let tree = Tree::new("write-programs");
    let root = tree.root();
    with_changes(&root);
    // Outside the server root, so that the scripts and their markers are none of its files.
    let tools = Scratch::new("write-programs-ran");
    let markers = programs::set_everywhere(&root, &tools.0);
    // Ocelli refuses what these would have git sign (ADR-0033); the writes here go through.
    let mut config = git2::Repository::open(&root).unwrap().config().unwrap();
    for (key, _) in programs::SWITCHES {
        config.remove(key).unwrap();
    }
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let head = git(&root, &["rev-parse", "HEAD"]);
    // Where git runs post-index-change, and the clean filter of every path; each write made from
    // what the repository shows once the one before it ran.
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
    // Where git runs pre-commit, prepare-commit-msg, commit-msg, the editor, and post-commit.
    let commit = Write::Commit {
        message: "from the browser".into(),
        head: Some(head),
    };
    let made = perform(&mut tab, &commit).await.unwrap();
    assert_eq!(made, json!(git(&root, &["rev-parse", "HEAD"])), "committed");
    let ran = markers.ran();
    assert!(ran.is_empty(), "programs ran: {ran:?}");
}

/// The packages a package of the lockfile depends on, by name.
fn lock_dependencies(lock: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut graph = std::collections::HashMap::new();
    for package in lock.split("[[package]]").skip(1) {
        let name = package
            .lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .map(|n| n.trim_end_matches('"').to_string())
            .unwrap();
        let deps = package
            .split_once("dependencies = [")
            .map(|(_, rest)| rest.split(']').next().unwrap())
            .unwrap_or("")
            .lines()
            .map(|l| l.trim().trim_matches(|c| c == '"' || c == ','))
            .filter(|l| !l.is_empty())
            .map(|l| l.split(' ').next().unwrap().to_string())
            .collect::<Vec<_>>();
        graph.entry(name).or_insert_with(Vec::new).extend(deps);
    }
    graph
}

/// Every package `from` reaches in the lockfile's tree, itself included.
fn reached(lock: &str, from: &str) -> std::collections::BTreeSet<String> {
    let graph = lock_dependencies(lock);
    let mut seen = std::collections::BTreeSet::new();
    let mut next = vec![from.to_string()];
    while let Some(name) = next.pop() {
        if seen.insert(name.clone()) {
            next.extend(graph.get(&name).cloned().unwrap_or_default());
        }
    }
    seen
}

fn desktop(name: &str) -> bool {
    name.starts_with("tauri") || name == "wry" || name.starts_with("webkit") || name == "gtk"
}

#[test]
// Verifies: ADR-0037
fn the_server_links_no_part_of_tauri_or_webkit() {
    let lock =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    let server = reached(&lock, "ocelli-server");
    assert!(
        server.contains("ocelli-git") && server.contains("axum"),
        "{server:?}"
    );
    let linked: Vec<_> = server.iter().filter(|n| desktop(n)).collect();
    assert!(linked.is_empty(), "ocelli-server links {linked:?}");
    // The scan sees them where they are: the desktop host links them.
    let host = reached(&lock, "ocelli");
    assert!(host.iter().any(|n| desktop(n)), "the desktop host's tree");
}

/// A server root inside a repository that encloses it, beside a repository outside it: the
/// tree the ways out of the server root are tried on.
struct Enclosed(
    PathBuf,
    #[allow(
        dead_code,
        reason = "held so that the tree is removed when the test ends"
    )]
    Scratch,
);

impl Enclosed {
    fn new(name: &str) -> Enclosed {
        let scratch = Scratch::new(&format!("enclosed-{name}"));
        let dir = scratch.0.clone();
        three_commits(&dir.join("encl"));
        three_commits(&dir.join("outside"));
        std::fs::create_dir_all(dir.join("encl/root")).unwrap();
        Enclosed(dir, scratch)
    }

    fn root(&self) -> PathBuf {
        self.0.join("encl/root")
    }

    fn outside(&self) -> PathBuf {
        self.0.join("outside")
    }
}

fn refused(path: &str) -> Result<Value, String> {
    Err(ocelli_server::root::refused(path))
}

/// One case of each kind of way out at the WebSocket: a `.git` file, an object store, a search
/// upwards, the root itself inside a repository, and `core.worktree`. Each way out of its kind
/// is a case of the confinement's own tests, `crates/git/tests/confine_checks.rs`.
#[tokio::test]
// Verifies: ADR-0039
async fn a_repository_below_the_root_that_reaches_outside_it_is_refused() {
    let tree = Enclosed::new("ways-out");
    let root = tree.root();
    let outside_git = tree.outside().join(".git");
    // (a) a `.git` file naming a git directory outside.
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(
        root.join("a/.git"),
        format!("gitdir: {}\n", outside_git.display()),
    )
    .unwrap();
    // (c) object stores borrowed from outside.
    three_commits(&root.join("c"));
    std::fs::write(
        root.join("c/.git/objects/info/alternates"),
        format!("{}\n", outside_git.join("objects").display()),
    )
    .unwrap();
    // (d) an empty `.git` folder, from which a search upwards finds the enclosing repository.
    std::fs::create_dir_all(root.join("d/.git")).unwrap();
    // (e) a working directory outside, named by `core.worktree`.
    three_commits(&root.join("e"));
    git(
        &root.join("e"),
        &["config", "core.worktree", &tree.outside().to_string_lossy()],
    );
    // A repository that lies below: what is refused could have opened.
    three_commits(&root.join("ok"));

    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    // The server root itself lies inside the enclosing repository and is none.
    assert_eq!(tab.invoke("startup_repo", json!({})).await, Ok(Value::Null));
    for path in ["a", "c", "d", "e", "."] {
        assert_eq!(
            tab.invoke("open_repo", json!({ "path": path })).await,
            refused(path),
            "{path}"
        );
    }
    assert!(
        !tab.events.iter().any(|(e, _)| e == "set-repo-label"),
        "nothing was opened: {:?}",
        tab.events
    );
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "ok" })).await,
        Ok(Value::Null)
    );
    tab.until("history-grew", |p| p["loading"] == false).await;
}

#[tokio::test]
// Verifies: ADR-0039, ADR-0037
async fn a_repository_swapped_for_a_way_out_after_it_opened_is_checked_again() {
    let tree = Enclosed::new("swap");
    let root = tree.root();
    three_commits(&root.join("s"));
    // A server that writes: a write is checked again as a read is.
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "s" }))
        .await
        .unwrap();
    tab.until("history-grew", |p| p["loading"] == false).await;
    let head = git(&root.join("s"), &["rev-parse", "HEAD"]);
    // Before the swap the commit's files and diff are read.
    assert_eq!(
        tab.invoke("select_commit", json!({ "sha": head })).await,
        Ok(Value::Null)
    );
    tab.until("commit-files", |_| true).await;
    assert!(
        tab.invoke("get_file_diff", json!({ "sha": head, "filePath": "f" }))
            .await
            .is_ok()
    );
    std::fs::write(root.join("s/f"), "staged\n").unwrap();
    let staged = stage_in(&root.join("s"), "f");
    assert_eq!(perform(&mut tab, &staged).await, Ok(Value::Null));

    std::fs::rename(root.join("s/.git"), root.join("s-git")).unwrap();
    std::fs::write(
        root.join("s/.git"),
        format!("gitdir: {}\n", tree.outside().join(".git").display()),
    )
    .unwrap();
    let outside_head = git(&tree.outside(), &["rev-parse", "HEAD"]);
    // The writes the server performs, each one the repository outside would take.
    std::fs::write(tree.outside().join("f"), "changed\n").unwrap();
    let outside = snapshot(&tree.outside());
    let stage = stage_in(&tree.outside(), "f");
    let commit = Write::Commit {
        message: "from the browser".into(),
        head: Some(outside_head.clone()),
    };
    for write in [stage, commit] {
        let sent = json!({ "write": write, "confirmed": false });
        assert_eq!(
            tab.invoke("perform_write", sent).await,
            refused("s"),
            "{write:?}"
        );
    }
    assert_eq!(snapshot(&tree.outside()), outside);
    for (cmd, args) in [
        ("select_commit", json!({ "sha": outside_head })),
        (
            "get_file_diff",
            json!({ "sha": outside_head, "filePath": "f" }),
        ),
        ("open_repo", json!({ "path": "s" })),
    ] {
        assert_eq!(tab.invoke(cmd, args).await, refused("s"), "{cmd}");
    }
}

#[tokio::test]
// Verifies: ADR-0039
async fn the_sidebar_names_no_path_outside_the_root() {
    let tree = Tree::new("sidebar");
    let root = tree.root();
    let outside_tree = tree.0.join("linked-outside");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "away",
            &outside_tree.to_string_lossy(),
        ],
    );
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "near", "inside"],
    );
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    let refs = tab.until("load-refs", |_| true).await;
    let mut paths: Vec<String> = refs["refs"]["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["path"].as_str().unwrap().to_string())
        .collect();
    paths.sort();
    assert_eq!(paths, ["", ".", "inside"]);
    let text = serde_json::to_string(&refs).unwrap();
    assert!(!text.contains(&*tree.0.to_string_lossy()), "{text}");
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn wrong_tokens_past_a_few_are_answered_ever_later_and_the_right_one_still_opens() {
    let tree = Tree::new("throttle");
    let running = server(&tree).await;
    let port = running.port;
    let wrong = format!("/?token={}", "0".repeat(64));
    // The first few are answered at once (`wait_after` in `auth.rs` holds how long).
    for _ in 0..ocelli_server::auth::FREE_FAILURES {
        assert_eq!(get(port, &wrong, &[]).await.status, 403);
    }
    // Then each waits longer: 250 ms, 500 ms, 1 s.
    for n in 0..3 {
        let begun = std::time::Instant::now();
        assert_eq!(get(port, &wrong, &[]).await.status, 403);
        assert!(begun.elapsed() >= ocelli_server::auth::FIRST_WAIT * 2u32.pow(n));
    }
    // The right token is not queued behind them: it is answered well before the 2 s the next
    // wrong one waits.
    let begun = std::time::Instant::now();
    let cookie = log_in(&running).await;
    assert!(begun.elapsed() < ocelli_server::auth::FIRST_WAIT * 8);
    assert_eq!(get(port, "/", &[("Cookie", &cookie)]).await.status, 200);
}

#[tokio::test]
// Verifies: ADR-0038
async fn a_connection_that_does_not_send_its_headers_in_time_is_closed() {
    let tree = Tree::new("slow");
    let limits = Limits {
        header_read: Duration::from_millis(300),
        ..Limits::default()
    };
    let running = start_on(tree.root(), limits).await;
    let mut stream = TcpStream::connect(("127.0.0.1", running.port))
        .await
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut rest)).await;
    assert!(closed.is_ok(), "the connection is still open");
}

#[tokio::test]
// Verifies: ADR-0037
async fn a_tab_that_does_not_read_its_answers_is_closed() {
    let tree = Tree::new("unread");
    let limits = Limits {
        queue: 4,
        send_wait: Duration::from_millis(200),
        ..Limits::default()
    };
    let running = start_on(tree.root(), limits).await;
    let mut tab = Tab::open(&running).await;
    // Each answer is a quarter of a megabyte; together far more than the connection buffers.
    let shas = vec!["a"; 50_000];
    let command = |id: u64| json!({ "id": id, "cmd": "rows_of", "args": { "shas": shas } });
    let sending = async {
        let mut sent = 0;
        for id in 1..=200 {
            let text = command(id).to_string();
            if tab.socket.send(Message::Text(text.into())).await.is_err() {
                break;
            }
            sent += 1;
        }
        let mut answered = 0;
        while let Some(Ok(message)) = tab.socket.next().await {
            if message.is_text() {
                answered += 1;
            }
        }
        (sent, answered)
    };
    let (sent, answered) = tokio::time::timeout(Duration::from_secs(60), sending)
        .await
        .expect("the server closed the connection");
    assert!(answered < sent, "{answered} of {sent} answered");
}

#[cfg(unix)]
#[tokio::test]
// Verifies: ADR-0039
async fn a_link_inside_the_git_directory_or_an_alternates_line_read_otherwise_is_refused() {
    // One case of each kind; the rest are cases of the confinement's own tests.
    let tree = Enclosed::new("inner-ways-out");
    let root = tree.root();
    git(&tree.outside(), &["gc", "--quiet"]);
    let outside_objects = tree.outside().join(".git/objects");
    // The object packs, a link out.
    three_commits(&root.join("pack"));
    let pack = root.join("pack/.git/objects/pack");
    std::fs::remove_dir_all(&pack).unwrap();
    std::os::unix::fs::symlink(outside_objects.join("pack"), &pack).unwrap();
    // A relative alternates line without the dot, which libgit2 reads against the process's
    // working directory, beside a decoy below the root where the line would lead from the store.
    three_commits(&root.join("relative"));
    let objects = root.join("relative/.git/objects");
    std::fs::create_dir_all(objects.join("outside/.git/objects")).unwrap();
    std::fs::write(objects.join("info/alternates"), "outside/.git/objects\n").unwrap();

    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    for path in ["pack", "relative"] {
        assert_eq!(
            tab.invoke("open_repo", json!({ "path": path })).await,
            refused(path),
            "{path}"
        );
    }
}

#[tokio::test]
// Verifies: ADR-0039
async fn the_sidebar_reads_no_superproject_or_submodule_outside_the_root() {
    let tree = Enclosed::new("sidebar-out");
    let root = tree.root();
    // The server root is a repository the enclosing one configures as its submodule.
    three_commits(&root);
    git(
        &tree.0.join("encl"),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            "./root",
            "root",
        ],
    );
    // A submodule of the root whose git directory is outside it.
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            &tree.outside().to_string_lossy(),
            "m",
        ],
    );
    git(&root, &["commit", "--quiet", "-m", "m"]);
    std::fs::write(
        root.join("m/.git"),
        format!("gitdir: {}\n", tree.outside().join(".git").display()),
    )
    .unwrap();
    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    let refs = tab.until("load-refs", |_| true).await;
    assert_eq!(refs["refs"]["parent"], Value::Null, "{refs}");
    let submodule = &refs["refs"]["submodules"][0];
    assert_eq!(submodule["path"], "m");
    assert_eq!(submodule["checkedOutShortSha"], Value::Null, "{submodule}");
    assert_eq!(submodule["workdir"], "");
    let outside_head = git(&tree.outside(), &["rev-parse", "--short", "HEAD"]);
    // The pinned commit is the superproject's own record; the checked-out one is not read.
    assert_eq!(submodule["shortSha"], outside_head.as_str());
}

#[tokio::test]
// Verifies: ADR-0039
async fn what_git_says_about_a_failed_read_reaches_the_browser_only_as_reading_failed() {
    let tree = Tree::new("git-says");
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    // Before a repository is open, the browser reads why nothing was read.
    for cmd in ["get_file_diff", "select_commit"] {
        assert_eq!(
            tab.invoke(cmd, json!({ "sha": "c", "filePath": "f" }))
                .await,
            Err(ocelli_commands::HostError::NoRepository.to_string()),
            "{cmd}"
        );
    }
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    tab.until("history-grew", |p| p["loading"] == false).await;
    assert_eq!(
        tab.invoke(
            "get_file_diff",
            json!({ "sha": "no-such-commit", "filePath": "f" })
        )
        .await,
        Err(ocelli_server::socket::READ_FAILED.to_string())
    );
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn connections_and_tabs_past_their_bound_are_refused() {
    let tree = Tree::new("bounds");
    let running = start_on(
        tree.root(),
        Limits {
            tabs: 1,
            ..Limits::default()
        },
    )
    .await;
    let _first = Tab::open(&running).await;
    let cookie = log_in(&running).await;
    let origin = format!("http://localhost:{}", running.port);
    assert_eq!(
        connect(running.port, Some(&cookie), Some(&origin))
            .await
            .err(),
        Some(503)
    );

    let running = start_on(
        tree.root(),
        Limits {
            connections: 1,
            ..Limits::default()
        },
    )
    .await;
    let idle = TcpStream::connect(("127.0.0.1", running.port))
        .await
        .unwrap();
    // The idle connection holds the one permit: the next is closed unanswered.
    let mut next = TcpStream::connect(("127.0.0.1", running.port))
        .await
        .unwrap();
    next.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .ok();
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), next.read_to_end(&mut answer)).await;
    assert!(answer.is_empty(), "{}", String::from_utf8_lossy(&answer));
    // Once it goes, a connection is served again.
    drop(idle);
    let mut served = false;
    for _ in 0..50 {
        let mut stream = TcpStream::connect(("127.0.0.1", running.port))
            .await
            .unwrap();
        let _ = stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await;
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer).await;
        if answer.starts_with(b"HTTP/1.1 403") {
            served = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(served);
}

#[cfg(unix)]
#[tokio::test]
// Verifies: ADR-0039
async fn a_linked_folder_or_a_path_leading_out_reads_nothing_outside_the_root() {
    let tree = Enclosed::new("workdir-link");
    let root = tree.root();
    let repo = root.join("w");
    three_commits(&repo);
    std::fs::create_dir_all(repo.join("d")).unwrap();
    std::fs::write(repo.join("d/secret"), "inside\n").unwrap();
    git(&repo, &["add", "d/secret"]);
    git(&repo, &["commit", "--quiet", "-m", "d"]);
    let folder = tree.0.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("secret"), "TOPSECRET\n").unwrap();
    let outside_blob = git(&folder, &["hash-object", "secret"]);
    // The tracked folder swapped for a link to a folder outside.
    std::fs::remove_dir_all(repo.join("d")).unwrap();
    std::os::unix::fs::symlink(&folder, repo.join("d")).unwrap();

    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "w" }))
        .await
        .unwrap();
    tab.until("history-grew", |p| p["loading"] == false).await;
    let workdir = ocelli_core::model::WORKDIR_SHA;
    assert_eq!(
        tab.invoke("select_commit", json!({ "sha": workdir })).await,
        Ok(Value::Null)
    );
    let files = tab.until("commit-files", |_| true).await;
    let files = files.as_str().unwrap();
    // The working directory shows the folder as the link git sees, and it changed.
    assert!(files.contains("\"d"), "{files}");
    assert!(!files.contains(&outside_blob), "{files}");
    let outside_secret = folder.join("secret").to_string_lossy().into_owned();
    for path in [
        "d/secret",
        "d",
        "../folder/secret",
        "../../folder/secret",
        outside_secret.as_str(),
    ] {
        for sha in [workdir.to_string(), git(&repo, &["rev-parse", "HEAD"])] {
            let answer = tab
                .invoke("get_file_diff", json!({ "sha": sha, "filePath": path }))
                .await;
            let text = format!("{answer:?}");
            assert!(!text.contains("TOPSECRET"), "{path}: {text}");
            assert!(!text.contains(&outside_blob), "{path}: {text}");
        }
    }
}

#[tokio::test]
// Verifies: ADR-0039
async fn a_git_directory_larger_than_the_walk_is_refused_as_too_large() {
    let tree = Enclosed::new("walk-cap");
    let root = tree.root();
    three_commits(&root.join("w"));
    let small = start_on(
        root.clone(),
        Limits {
            walk: 5,
            ..Limits::default()
        },
    )
    .await;
    let mut tab = Tab::open(&small).await;
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "w" })).await,
        Err(ocelli_server::root::too_large("w"))
    );
    let default = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&default).await;
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "w" })).await,
        Ok(Value::Null)
    );
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn an_upgrade_without_a_token_or_with_a_wrong_one_is_refused() {
    let tree = Tree::new("upgrade-token");
    let running = server(&tree).await;
    let port = running.port;
    let origin = format!("http://localhost:{port}");
    // The cookie the right token gives opens the socket that is refused without it.
    let cookie = log_in(&running).await;
    assert!(connect(port, Some(&cookie), Some(&origin)).await.is_ok());

    let wrong = format!("/ws?token={}", "0".repeat(64));
    assert_eq!(
        handshake(port, "/ws", None, Some(&origin)).await.err(),
        Some(403)
    );
    assert_eq!(
        handshake(port, &wrong, None, Some(&origin)).await.err(),
        Some(403)
    );
    // A wrong token refuses the upgrade even beside a valid cookie: it is not this server's.
    assert_eq!(
        handshake(port, &wrong, Some(&cookie), Some(&origin))
            .await
            .err(),
        Some(403)
    );
    // The right token on the socket's path is exchanged, never upgraded.
    let token = target(&running).replacen("/?", "/ws?", 1);
    assert_eq!(
        handshake(port, &token, None, Some(&origin)).await.err(),
        Some(303)
    );
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn a_session_the_server_no_longer_keeps_or_a_previous_start_gave_is_refused() {
    let tree = Tree::new("expired");
    let running = start_on(
        tree.root(),
        Limits {
            sessions: 1,
            ..Limits::default()
        },
    )
    .await;
    let port = running.port;
    let origin = format!("http://localhost:{port}");
    let first = log_in(&running).await;
    assert_ne!(get(port, "/", &[("Cookie", &first)]).await.status, 403);
    assert!(connect(port, Some(&first), Some(&origin)).await.is_ok());
    // The second session takes the place of the first, which has expired.
    let second = log_in(&running).await;
    assert_eq!(get(port, "/", &[("Cookie", &first)]).await.status, 403);
    assert_eq!(
        connect(port, Some(&first), Some(&origin)).await.err(),
        Some(403)
    );
    assert_ne!(get(port, "/", &[("Cookie", &second)]).await.status, 403);
    assert!(connect(port, Some(&second), Some(&origin)).await.is_ok());

    // A new start keeps none of the sessions of the one before.
    let restarted = server(&tree).await;
    let port = restarted.port;
    let origin = format!("http://localhost:{port}");
    assert_eq!(get(port, "/", &[("Cookie", &second)]).await.status, 403);
    assert_eq!(
        connect(port, Some(&second), Some(&origin)).await.err(),
        Some(403)
    );
    let fresh = log_in(&restarted).await;
    assert!(connect(port, Some(&fresh), Some(&origin)).await.is_ok());
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn a_request_or_upgrade_with_a_valid_cookie_from_any_other_origin_is_refused() {
    let tree = Tree::new("origins");
    let running = server(&tree).await;
    let port = running.port;
    let cookie = log_in(&running).await;
    let own = format!("http://localhost:{port}");
    let same = get(port, "/", &[("Cookie", &cookie), ("Origin", &own)]).await;
    assert_ne!(same.status, 403);
    assert!(connect(port, Some(&cookie), Some(&own)).await.is_ok());
    let foreign = [
        "http://evil.example".to_string(),
        "null".to_string(),
        format!("https://localhost:{port}"),
        format!("http://localhost:{}", if port == 1 { 2 } else { port - 1 }),
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}.evil.example"),
        format!("http://evil.example/{own}"),
    ];
    for origin in &foreign {
        let answer = get(port, "/", &[("Cookie", &cookie), ("Origin", origin)]).await;
        assert_eq!(answer.status, 403, "{origin}");
        assert_eq!(answer.body, "Requests from another origin are refused");
        assert_eq!(
            connect(port, Some(&cookie), Some(origin)).await.err(),
            Some(403),
            "{origin}"
        );
    }
    // A page of another origin cannot log the browser in with the token either.
    let login = get(
        port,
        &target(&running),
        &[("Origin", "http://evil.example")],
    )
    .await;
    assert_eq!(login.status, 403);
    assert_eq!(login.header("set-cookie"), None);
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn wrong_tokens_from_many_connections_wait_their_turn_for_the_whole_server() {
    let tree = Tree::new("throttle-shared");
    let running = server(&tree).await;
    let port = running.port;
    let origin = format!("http://localhost:{port}");
    let wrong = "0".repeat(64);
    // The free failures spent on the socket's path count as any others.
    let on_socket = format!("/ws?token={wrong}");
    for _ in 0..ocelli_server::auth::FREE_FAILURES {
        assert_eq!(
            handshake(port, &on_socket, None, Some(&origin)).await.err(),
            Some(403)
        );
    }
    // Three at once, from three connections: each is refused, one after the other, each after
    // the wait its place in the count earned (250 ms, 500 ms, 1 s).
    let begun = std::time::Instant::now();
    let target = format!("/?token={wrong}");
    let (a, b, c) = tokio::join!(
        get(port, &target, &[]),
        get(port, &target, &[]),
        get(port, &target, &[])
    );
    for answer in [&a, &b, &c] {
        assert_eq!(answer.status, 403);
        assert_eq!(answer.header("set-cookie"), None);
    }
    let first = ocelli_server::auth::FIRST_WAIT;
    assert!(begun.elapsed() >= first + first * 2 + first * 4);
    // The right token starts the count again, and a valid cookie carries no wrong token through.
    let cookie = log_in(&running).await;
    let begun = std::time::Instant::now();
    let answer = get(port, &target, &[("Cookie", &cookie)]).await;
    assert_eq!(answer.status, 403);
    assert_eq!(answer.header("set-cookie"), None);
    assert!(begun.elapsed() < first);
}

/// The relative path of the sidebar row of `refs` under `list` whose `key` is `value`.
fn row<'a>(refs: &'a Value, list: &str, key: &str, value: &str) -> &'a Value {
    refs["refs"][list]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[key] == value)
        .unwrap_or_else(|| panic!("no {list} row {value} in {refs}"))
}

#[tokio::test]
// Verifies: ADR-0039, ADR-0027
async fn a_submodule_and_a_worktree_below_the_root_open_through_their_rows_and_lead_back() {
    let tree = Tree::new("switch");
    let root = tree.root();
    // A submodule cloned from the repository outside: its git directory lies in the root's
    // `.git/modules`, below the server root.
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            &tree.0.join("outside").to_string_lossy(),
            "sm",
        ],
    );
    git(&root, &["commit", "--quiet", "-m", "sm"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "near", "inside"],
    );
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    let refs = open_and_load(&mut tab, ".").await;
    let submodule = row(&refs, "submodules", "path", "sm")["workdir"].clone();
    assert_eq!(submodule, "sm");
    let worktree = row(&refs, "worktrees", "branch", "near")["path"].clone();
    assert_eq!(worktree, "inside");

    // The submodule's row opens it, and its parent row leads back to the server root.
    let refs = open_and_load(&mut tab, submodule.as_str().unwrap()).await;
    let outside_head = git(&tree.0.join("outside"), &["rev-parse", "HEAD"]);
    let window = tab
        .invoke("read_window", json!({ "top": 0, "rows": 10 }))
        .await
        .unwrap();
    assert_eq!(window["nodes"][0]["sha"], outside_head.as_str());
    assert_eq!(refs["refs"]["parent"]["kind"], "superproject");
    assert_eq!(refs["refs"]["parent"]["path"], ".");
    open_and_load(&mut tab, ".").await;

    // The worktree's row opens it, its parent row the main working tree.
    let refs = open_and_load(&mut tab, worktree.as_str().unwrap()).await;
    assert_eq!(refs["refs"]["parent"]["kind"], "mainWorktree");
    assert_eq!(refs["refs"]["parent"]["path"], ".");
    assert_eq!(row(&refs, "worktrees", "branch", "near")["isCurrent"], true);
    open_and_load(&mut tab, ".").await;
}

#[tokio::test]
// Verifies: ADR-0039
async fn each_way_out_of_the_root_is_refused_with_the_reason_that_names_it() {
    let tree = Tree::new("reasons");
    let root = tree.root();
    std::fs::create_dir_all(root.join("dir")).unwrap();
    let outside = tree.0.join("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("dir/link")).unwrap();
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    let mut paths = vec![
        "..".to_string(),
        "../outside".to_string(),
        "dir/../../outside".to_string(),
        "./dir/../../outside/.".to_string(),
        outside.to_string_lossy().into_owned(),
        "missing".to_string(),
        "dir".to_string(),
    ];
    if cfg!(unix) {
        paths.extend(["dir/link".to_string(), "dir/link/.".to_string()]);
    }
    for path in &paths {
        assert_eq!(
            tab.invoke("open_repo", json!({ "path": path })).await,
            refused(path),
            "{path}"
        );
    }
    let said: Vec<_> = tab.events.iter().map(|(e, _)| e.clone()).collect();
    assert!(said.is_empty(), "nothing opened, nothing said: {said:?}");
}

/// A repository of `n` commits at `repo`, each its own message, so that its commits are no other
/// repository's.
fn commits(repo: &Path, n: usize, name: &str) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "--quiet", "-b", "main"]);
    for i in 1..=n {
        std::fs::write(repo.join(name), format!("{name} {i}\n")).unwrap();
        git(repo, &["add", name]);
        git(repo, &["commit", "--quiet", "-m", &format!("{name} {i}")]);
    }
}

/// The first fifty rows `tab` reads.
async fn window(tab: &mut Tab) -> Value {
    tab.invoke("read_window", json!({ "top": 0, "rows": 50 }))
        .await
        .unwrap()
}

/// A repository of `n` commits with an empty tree at `repo`, made at once by `git fast-import`.
fn many_commits(repo: &Path, n: usize) {
    use std::io::Write as _;
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "--quiet", "-b", "main"]);
    let mut stream = String::new();
    for i in 1..=n {
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{i}\ncommitter F <f@example.invalid> {} +0000\ndata 1\nc\n",
            1_000_000 + i
        ));
        if i > 1 {
            stream.push_str(&format!("from :{}\n", i - 1));
        }
        stream.push('\n');
    }
    let mut child = git_command(repo)
        .args(["fast-import", "--quiet"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

/// Waits until the server holds `n` load sessions, failing after 30 s.
async fn sessions_become(running: &Running, n: usize) {
    let waited = tokio::time::timeout(Duration::from_secs(30), async {
        while running.sessions() != n {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "{} sessions held, not {n}",
        running.sessions()
    );
}

#[tokio::test]
// Verifies: ADR-0037, G-8
async fn two_tabs_on_two_repositories_each_show_their_own_and_a_selection_stays_in_its_tab() {
    let tree = Tree::new("two-tabs");
    let root = tree.root();
    commits(&root.join("other"), 5, "o");
    // Ignored, so the root's working directory is clean and has no node of its own.
    std::fs::write(root.join(".git/info/exclude"), "/other/\n").unwrap();
    let running = server(&tree).await;
    let mut one = Tab::open(&running).await;
    let mut two = Tab::open(&running).await;
    one.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    two.invoke("open_repo", json!({ "path": "other" }))
        .await
        .unwrap();
    one.until("history-grew", |p| p["loading"] == false).await;
    two.until("history-grew", |p| p["loading"] == false).await;
    assert_eq!(running.sessions(), 2);

    let first = window(&mut one).await;
    let second = window(&mut two).await;
    assert_eq!(first["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(second["nodes"].as_array().unwrap().len(), 5);
    assert_eq!(
        first["nodes"][0]["sha"],
        git(&root, &["rev-parse", "HEAD"]).as_str()
    );
    assert_eq!(
        second["nodes"][0]["sha"],
        git(&root.join("other"), &["rev-parse", "HEAD"]).as_str()
    );
    assert_eq!(first, desktop_window(&root, 0, 50).await);
    assert_eq!(second, desktop_window(&root.join("other"), 0, 50).await);
    // Each tab heard only of its own repository.
    let labels = |tab: &Tab| -> Vec<Value> {
        tab.events
            .iter()
            .filter(|(e, _)| e == "set-repo-label")
            .map(|(_, p)| p.clone())
            .collect()
    };
    assert_eq!(labels(&one), [json!(".")]);
    assert_eq!(labels(&two), [json!("other")]);

    // A selection in one changes nothing in the other.
    let selected = one.invoke("select_row", json!({ "row": 1 })).await.unwrap();
    assert_eq!(selected["node"]["sha"], first["nodes"][1]["sha"]);
    assert_ne!(window(&mut one).await, first, "the lineage lights up");
    assert_eq!(two.invoke("selection", json!({})).await, Ok(Value::Null));
    assert_eq!(window(&mut two).await, second);
    // A search in one finds in its own history alone.
    let found = two
        .invoke("search_history", json!({ "query": "o 4" }))
        .await;
    assert_eq!(found.unwrap()["rows"], json!([1]));
    let found = one
        .invoke("search_history", json!({ "query": "o 4" }))
        .await;
    assert_eq!(found.unwrap()["rows"], json!([]));

    // Closing one tab drops its session; the other goes on.
    one.socket.close(None).await.unwrap();
    sessions_become(&running, 1).await;
    assert_eq!(window(&mut two).await, second);
}

#[tokio::test]
// Verifies: ADR-0037
async fn a_tab_closed_or_dropped_during_a_load_has_its_session_dropped() {
    let tree = Tree::new("close-loading");
    let root = tree.root();
    many_commits(&root.join("big"), 60_000);
    let running = server(&tree).await;
    for drop_it in [false, true] {
        let mut tab = Tab::open(&running).await;
        tab.invoke("open_repo", json!({ "path": "big" }))
            .await
            .unwrap();
        // Closed while the load runs: what is held is the tab's and its load's, and the count falls
        // only once both have let go. That the load is stopped rather than run to its end this
        // test does not tell apart, a load of this size ending within a second or two either way.
        tab.until("history-grew", |p| p["loading"] == true).await;
        assert_eq!(running.sessions(), 1);
        if drop_it {
            // The connection goes without a close: the browser crashed, or the network did.
            drop(tab);
        } else {
            tab.socket.close(None).await.unwrap();
        }
        sessions_become(&running, 0).await;
    }
}

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
/// hosts carry, never report (G-8, ADR-0037).
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
// Verifies: G-8, ADR-0037
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
// Verifies: G-8, ADR-0037
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
    let failed = || Err(ocelli_server::socket::WRITE_FAILED.to_string());
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
// Verifies: ADR-0037, G-8, Q-5
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

#[tokio::test]
// Verifies: ADR-0037, ADR-0036, ADR-0031, G-8
async fn each_read_the_browser_asks_is_answered_as_the_desktop_host_answers_it() {
    let tree = Tree::new("same-answers");
    let root = tree.root();
    git(&root, &["branch", "side", "HEAD~1"]);
    git(&root, &["tag", "-a", "v1", "-m", "v1", "HEAD~2"]);
    std::fs::write(root.join("f"), "stashed\n").unwrap();
    git(&root, &["stash", "push", "--quiet", "-m", "kept"]);
    std::fs::write(root.join("f"), "changed\n").unwrap();
    std::fs::write(root.join("new"), "new\n").unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]);
    let stash = git(&root, &["rev-parse", "stash@{0}"]);
    let first = git(&root, &["rev-list", "--max-parents=0", "HEAD"]);
    let workdir = ocelli_core::model::WORKDIR_SHA;

    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    tab.until("history-grew", |p| p["loading"] == false).await;
    let refs = tab.until("load-refs", |_| true).await;
    let (mut desktop, desktop_refs) = Desktop::open(&root).await;

    // The sidebar and the toolbar: the same refs and transfers; a working tree's path is the
    // one thing told otherwise, relative to the server root (ADR-0039).
    assert_eq!(refs["transfers"], desktop_refs["transfers"]);
    for key in [
        "locals",
        "remotes",
        "tags",
        "submodules",
        "parent",
        "inProgress",
        "defaultRemote",
    ] {
        assert_eq!(refs["refs"][key], desktop_refs["refs"][key], "{key}");
    }
    assert_eq!(refs["refs"]["tags"][0]["name"], "v1");
    let mut worktree = desktop_refs["refs"]["worktrees"][0].clone();
    worktree["path"] = json!(".");
    assert_eq!(refs["refs"]["worktrees"], json!([worktree]));

    // The windows, with the working-directory and stash nodes among their rows.
    let window = tab
        .invoke("read_window", json!({ "top": 0, "rows": 50 }))
        .await
        .unwrap();
    assert_eq!(window, to_value(desktop.host.read_window(0, 50)));
    let shas: Vec<&str> = window["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["sha"].as_str().unwrap())
        .collect();
    for sha in [workdir, stash.as_str(), head.as_str(), first.as_str()] {
        assert!(shas.contains(&sha), "{sha} in {shas:?}");
    }
    assert_eq!(
        tab.invoke("read_window", json!({ "top": 2, "rows": 2 }))
            .await
            .unwrap(),
        to_value(desktop.host.read_window(2, 2))
    );

    // Search, and the rows of SHAs.
    for query in ["commit 2", "kept", "Ada", "no such text"] {
        assert_eq!(
            tab.invoke("search_history", json!({ "query": query }))
                .await
                .unwrap(),
            to_value(desktop.host.search_history(query)),
            "{query}"
        );
    }
    let asked = vec![head.clone(), stash.clone(), workdir.into(), "nope".into()];
    assert_eq!(
        tab.invoke("rows_of", json!({ "shas": asked }))
            .await
            .unwrap(),
        to_value(desktop.host.rows_of(&asked))
    );

    // Selecting each row: the selection with its operations, and the lineage in the window.
    for row in 0..shas.len() {
        assert_eq!(
            tab.invoke("select_row", json!({ "row": row }))
                .await
                .unwrap(),
            to_value(desktop.host.select_row(row)),
            "row {row}"
        );
        assert_eq!(
            tab.invoke("selection", json!({})).await.unwrap(),
            to_value(desktop.host.selection())
        );
        assert_eq!(
            tab.invoke("read_window", json!({ "top": 0, "rows": 50 }))
                .await
                .unwrap(),
            to_value(desktop.host.read_window(0, 50))
        );
    }
    tab.invoke("deselect", json!({})).await.unwrap();
    desktop.host.deselect();
    assert_eq!(tab.invoke("selection", json!({})).await, Ok(Value::Null));
    assert_eq!(
        tab.invoke("read_window", json!({ "top": 0, "rows": 50 }))
            .await
            .unwrap(),
        to_value(desktop.host.read_window(0, 50))
    );

    // The changed files of a commit, the stash, and the working directory, and each file's diff.
    for sha in [head.as_str(), first.as_str(), stash.as_str(), workdir] {
        tab.events.clear();
        tab.invoke("select_commit", json!({ "sha": sha }))
            .await
            .unwrap();
        let files = tab.until("commit-files", |_| true).await;
        assert_eq!(files, desktop.commit_files(sha).await, "{sha}");
        let files: Value = serde_json::from_str(files.as_str().unwrap()).unwrap();
        let files = files.as_array().unwrap();
        assert!(!files.is_empty(), "{sha}");
        for file in files {
            let path = file["path"].as_str().unwrap();
            assert_eq!(
                tab.invoke("get_file_diff", json!({ "sha": sha, "filePath": path }))
                    .await,
                desktop
                    .host
                    .get_file_diff(sha.to_string(), path.to_string())
                    .await
                    .map(Value::String)
                    .map_err(String::from),
                "{sha} {path}"
            );
        }
    }

    // What a write would discard, which the web interface asks before it offers one.
    for write in [
        json!({ "kind": "createTag", "name": "t", "commit": head }),
        json!({ "kind": "deleteBranch", "name": "side", "tip": head, "merged": false }),
    ] {
        let parsed: ocelli_core::port::Write = serde_json::from_value(write.clone()).unwrap();
        assert_eq!(
            tab.invoke("write_discards", json!({ "write": write }))
                .await
                .unwrap(),
            to_value(Host::write_discards(&parsed))
        );
    }
}

// The twin is copied with `cp`, as the writer's own tests, which run on unix, copy theirs.
#[cfg(unix)]
#[tokio::test]
// Verifies: ADR-0037, ADR-0033, G-8, G-4
async fn with_allow_writes_staging_unstaging_and_committing_leave_the_repository_as_the_desktop_does()
 {
    let tree = Tree::new("writes");
    let root = tree.root();
    with_changes(&root);
    // The desktop's copy, which the shared Host writes as the desktop host does.
    let twin = support::twin(&tree.1, "writes-desktop");
    let other = twin.0.join("root");
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let (mut desktop, _) = Desktop::open(&other).await;
    let head = git(&root, &["rev-parse", "HEAD"]);
    let mut made = None;
    for step in 0..4 {
        // Each write as the web interface makes it from what it shows now.
        let write = match step {
            0 => stage_in(&root, "f"),
            1 => stage_in(&root, "new"),
            2 => unstage_in(&root, "new"),
            _ => Write::Commit {
                message: "from the browser".into(),
                head: Some(head.clone()),
            },
        };
        let (here, there) = (
            perform(&mut tab, &write).await,
            desktop.perform(&write).await,
        );
        match (&here, &there) {
            (Ok(Value::Null), Ok(Value::Null)) => {}
            (Ok(Value::String(a)), Ok(Value::String(b))) => made = Some((a.clone(), b.clone())),
            _ => panic!("{write:?}: the server answers {here:?}, the desktop host {there:?}"),
        }
        support::assert_twins_alike(&root, &other, &format!("{write:?}"));
    }
    let (here, there) = made.expect("the commit answers the commit it made");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), here);
    // As the user the server runs as, with the identity the repository's configuration names.
    assert_eq!(
        git(&root, &["log", "-1", "--format=%an <%ae>"]),
        "Ada <ada@example.invalid>"
    );
    // The load after the write selects the commit it made, as the desktop's does (ADR-0033).
    tab.events.clear();
    let reload = json!({ "path": ".", "select": here });
    assert_eq!(tab.invoke("open_repo", reload).await, Ok(Value::Null));
    let grew = tab.until("history-grew", |p| !p["select"].is_null()).await;
    assert_eq!(grew["select"], desktop.reload(&other, &there).await);
    let selected = tab
        .invoke("select_row", json!({ "row": grew["select"] }))
        .await
        .unwrap();
    assert_eq!(selected["node"]["sha"], json!(here));
}

#[tokio::test]
// Verifies: ADR-0039, ADR-0037
async fn the_browser_chooses_a_repository_from_the_folders_below_the_root() {
    let tree = Enclosed::new("choose");
    let root = tree.root();
    three_commits(&root.join("group/app"));
    std::fs::create_dir_all(root.join("group/plain")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(tree.outside(), root.join("group/away")).unwrap();
    let running = start_on(root.clone(), Limits::default()).await;
    let mut tab = Tab::open(&running).await;
    // The desktop's dialog is not answered under its name: the listing is its counterpart.
    assert_eq!(
        tab.invoke("browse_repo", json!({})).await,
        Err("Unknown command: browse_repo".to_string())
    );
    let listing = tab
        .invoke("list_folders", json!({ "path": "." }))
        .await
        .unwrap();
    assert_eq!(
        listing,
        json!({ "path": ".", "repository": false, "more": false, "folders": [
            { "name": "group", "path": "group", "repository": false },
        ]})
    );
    let listing = tab
        .invoke("list_folders", json!({ "path": "group" }))
        .await
        .unwrap();
    // The link out of the server root is not listed, though it leads to a repository.
    assert_eq!(
        listing["folders"],
        json!([
            { "name": "app", "path": "group/app", "repository": true },
            { "name": "plain", "path": "group/plain", "repository": false },
        ])
    );
    let outside = tree.outside().to_string_lossy().into_owned();
    let mut refused = vec!["..", "group/../..", outside.as_str()];
    if cfg!(unix) {
        refused.push("group/away");
    }
    for path in refused {
        assert_eq!(
            tab.invoke("list_folders", json!({ "path": path })).await,
            Err(ocelli_server::root::no_folder(path)),
            "{path}"
        );
    }
    // What is marked opens through the listing's path.
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": "group/app" }))
            .await,
        Ok(Value::Null)
    );
    tab.until("history-grew", |p| p["loading"] == false).await;
}

/// A server on `address` over TLS: with the developer's certificate and key, or a self-signed
/// one kept in `keep_in`.
async fn start_tls(
    root: PathBuf,
    address: &str,
    cert: Option<(PathBuf, PathBuf)>,
    keep_in: &Path,
) -> Result<Running, String> {
    start(Options {
        root,
        port: 0,
        limits: Limits::default(),
        listen: Some(ocelli_server::Listen {
            address: address.parse().unwrap(),
            cert,
            keep_in: Some(keep_in.to_path_buf()),
        }),
        allow_writes: false,
    })
    .await
}

/// The first certificate of the PEM file `path`.
fn pem_cert(path: &Path) -> rustls::pki_types::CertificateDer<'static> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::CertificateDer::from_pem_file(path).unwrap()
}

/// A TLS connection to `port` on 127.0.0.1 that trusts `cert` alone, checked for `name`.
async fn tls_connect(
    port: u16,
    cert: &rustls::pki_types::CertificateDer<'static>,
    name: &str,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    tls_connect_to(&format!("127.0.0.1:{port}"), cert, name).await
}

/// A TLS connection to `address` that trusts `cert` alone, checked for `name`.
async fn tls_connect_to(
    address: &str,
    cert: &rustls::pki_types::CertificateDer<'static>,
    name: &str,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.clone()).unwrap();
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let stream = TcpStream::connect(address).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from(name.to_string()).unwrap();
    connector.connect(name, stream).await.unwrap()
}

/// `GET target` over TLS to `port`, trusting `cert` for 127.0.0.1, under the `Host` given.
async fn tls_get(
    port: u16,
    cert: &rustls::pki_types::CertificateDer<'static>,
    host: &str,
    target: &str,
    extra: &[(&str, &str)],
) -> Answer {
    get_over(
        tls_connect(port, cert, "127.0.0.1").await,
        host,
        target,
        extra,
    )
    .await
}

/// A WebSocket handshake over TLS with the cookie and origin given; the status of a refusal.
async fn tls_handshake(
    port: u16,
    cert: &rustls::pki_types::CertificateDer<'static>,
    cookie: &str,
    origin: &str,
) -> Result<(), u16> {
    let mut request = format!("wss://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", HeaderValue::from_str(cookie).unwrap());
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_str(origin).unwrap());
    let stream = tls_connect(port, cert, "127.0.0.1").await;
    match tokio_tungstenite::client_async(request, stream).await {
        Ok(_) => Ok(()),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(e) => panic!("the handshake failed otherwise: {e}"),
    }
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn with_listen_and_no_certificate_one_is_made_kept_for_the_developer_and_reused() {
    let tree = Tree::new("self-signed");
    let keep_in = tree.0.join("config/ocelli/server");
    let running = start_tls(tree.root(), "127.0.0.1:0", None, &keep_in)
        .await
        .unwrap();
    let port = running.port;
    assert_eq!(
        running.addresses,
        [format!("127.0.0.1:{port}").parse().unwrap()]
    );
    assert!(
        running
            .url
            .starts_with(&format!("https://127.0.0.1:{port}/?token=")),
        "{}",
        running.url
    );
    let (cert_file, key_file) = (keep_in.join("cert.pem"), keep_in.join("key.pem"));
    let cert = pem_cert(&cert_file);
    assert_eq!(
        running.fingerprint,
        Some(ocelli_server::tls::fingerprint(&cert))
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(&keep_in), 0o700);
        assert_eq!(mode(&cert_file), 0o600);
        assert_eq!(mode(&key_file), 0o600);
    }

    // The token is exchanged over TLS for a cookie that is `Secure` and carries the prefix.
    let target = running.url.split_once(&format!(":{port}")).unwrap().1;
    let host = format!("127.0.0.1:{port}");
    let answer = tls_get(port, &cert, &host, target, &[]).await;
    assert_eq!(answer.status, 303);
    let set = answer.header("set-cookie").unwrap().to_string();
    for part in [
        "__Host-ocelli-session=",
        "HttpOnly",
        "SameSite=Strict",
        "Path=/",
        "Secure",
    ] {
        assert!(set.contains(part), "{part} in {set}");
    }
    assert!(!set.contains("Domain") && !set.contains("Expires"), "{set}");
    let cookie = set.split(';').next().unwrap().to_string();
    let page = tls_get(port, &cert, &host, "/", &[("Cookie", &cookie)]).await;
    assert_ne!(page.status, 403);
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains(&format!("connect-src 'self' wss://{host};"))
    );
    // The name without the prefix is not the cookie over TLS.
    let unprefixed = cookie.replace("__Host-", "");
    let refused = tls_get(port, &cert, &host, "/", &[("Cookie", &unprefixed)]).await;
    assert_eq!(refused.status, 403);
    // A name the certificate does not carry is refused, as a page through DNS rebinding sends.
    let rebound = format!("evil.example:{port}");
    let refused = tls_get(port, &cert, &rebound, "/", &[("Cookie", &cookie)]).await;
    assert_eq!(refused.status, 403);
    // The socket opens from the server's own origin, which is `https`, and from no other.
    let own = format!("https://{host}");
    assert_eq!(tls_handshake(port, &cert, &cookie, &own).await, Ok(()));
    let plain = format!("http://{host}");
    assert_eq!(tls_handshake(port, &cert, &cookie, &plain).await, Err(403));

    // A second start reuses the certificate kept, and keeps none of the first start's sessions.
    let (cert_text, key_text) = (
        std::fs::read(&cert_file).unwrap(),
        std::fs::read(&key_file).unwrap(),
    );
    let again = start_tls(tree.root(), "127.0.0.1:0", None, &keep_in)
        .await
        .unwrap();
    assert_eq!(again.fingerprint, running.fingerprint);
    assert_eq!(std::fs::read(&cert_file).unwrap(), cert_text);
    assert_eq!(std::fs::read(&key_file).unwrap(), key_text);
    let host = format!("127.0.0.1:{}", again.port);
    let refused = tls_get(again.port, &cert, &host, "/", &[("Cookie", &cookie)]).await;
    assert_eq!(refused.status, 403);
    // One kept for another address is refused, not replaced.
    let other = start_tls(tree.root(), "[::1]:0", None, &keep_in).await;
    assert!(
        other.as_ref().is_err_and(|e| e.contains("is not for ::1")),
        "{:?}",
        other.err()
    );
    assert_eq!(std::fs::read(&cert_file).unwrap(), cert_text);
}

#[tokio::test]
// Verifies: ADR-0038
async fn with_listen_and_a_certificate_and_key_given_it_serves_those() {
    let tree = Tree::new("given-cert");
    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let (cert_file, key_file) = (tree.0.join("given.pem"), tree.0.join("given.key"));
    std::fs::write(&cert_file, made.cert.pem()).unwrap();
    std::fs::write(&key_file, made.signing_key.serialize_pem()).unwrap();
    let keep_in = tree.0.join("config");
    let running = start_tls(
        tree.root(),
        "127.0.0.1:0",
        Some((cert_file.clone(), key_file)),
        &keep_in,
    )
    .await
    .unwrap();
    let cert = made.cert.der().clone();
    assert_eq!(
        running.fingerprint,
        Some(ocelli_server::tls::fingerprint(&cert))
    );
    assert!(
        !keep_in.exists(),
        "nothing is made where a certificate is given"
    );
    // A client that trusts the given certificate alone completes the handshake for its name.
    let port = running.port;
    let target = running.url.split_once(&format!(":{port}")).unwrap().1;
    let stream = tls_connect(port, &cert, "localhost").await;
    let answer = get_over(stream, &format!("localhost:{port}"), target, &[]).await;
    assert_eq!(answer.status, 303);
    assert!(
        answer
            .header("set-cookie")
            .unwrap()
            .starts_with("__Host-ocelli-session=")
    );
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn with_listen_plain_http_is_not_answered() {
    let tree = Tree::new("plain-on-tls");
    let keep_in = tree.0.join("config");
    let running = start_tls(tree.root(), "127.0.0.1:0", None, &keep_in)
        .await
        .unwrap();
    // That the listener answers over TLS.
    let cert = pem_cert(&keep_in.join("cert.pem"));
    let host = format!("127.0.0.1:{}", running.port);
    assert_eq!(
        tls_get(running.port, &cert, &host, "/", &[]).await.status,
        403
    );
    let mut stream = TcpStream::connect(("127.0.0.1", running.port))
        .await
        .unwrap();
    let target = running
        .url
        .split_once(&format!(":{}", running.port))
        .unwrap()
        .1;
    let request = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut answer)).await;
    assert!(
        !String::from_utf8_lossy(&answer).contains("HTTP/"),
        "{}",
        String::from_utf8_lossy(&answer)
    );
}

#[tokio::test]
// Verifies: ADR-0038
async fn by_default_the_server_listens_on_the_loopback_addresses_alone() {
    let tree = Tree::new("loopback");
    let running = server(&tree).await;
    assert!(!running.addresses.is_empty());
    for address in &running.addresses {
        assert!(address.ip().is_loopback(), "{address}");
        assert_eq!(address.port(), running.port);
    }
    assert_eq!(running.fingerprint, None);
    assert!(running.url.starts_with("http://localhost:"));
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn wrong_tokens_that_wait_hold_no_connection_and_never_keep_the_right_token_out() {
    let tree = Tree::new("throttle-slots");
    let connections = 8;
    let running = start_on(
        tree.root(),
        Limits {
            connections,
            ..Limits::default()
        },
    )
    .await;
    let port = running.port;
    let wrong = format!("/?token={}", "0".repeat(64));
    for _ in 0..ocelli_server::auth::FREE_FAILURES {
        assert_eq!(get(port, &wrong, &[]).await.status, 403);
    }
    // As many wrong tokens as the server serves connections at once, each now waiting its turn.
    let waiting: Vec<_> = (0..connections)
        .map(|_| {
            let wrong = wrong.clone();
            tokio::spawn(async move { get(port, &wrong, &[]).await.status })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The right token is answered at once, beside them.
    let begun = std::time::Instant::now();
    let answer = get(port, &target(&running), &[]).await;
    assert_eq!(answer.status, 303);
    assert!(begun.elapsed() < ocelli_server::auth::FIRST_WAIT);
    // The first of them has been answered meanwhile, refused.
    let first = waiting.into_iter().next().unwrap();
    assert_eq!(first.await.unwrap(), 403);
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn a_wrong_token_past_those_waiting_is_refused_at_once_with_when_to_try_again() {
    let tree = Tree::new("throttle-waiting");
    let running = start_on(
        tree.root(),
        Limits {
            waiting: 2,
            ..Limits::default()
        },
    )
    .await;
    let port = running.port;
    let wrong = format!("/?token={}", "0".repeat(64));
    for _ in 0..ocelli_server::auth::FREE_FAILURES {
        assert_eq!(get(port, &wrong, &[]).await.status, 403);
    }
    let (a, b, c) = tokio::join!(
        get(port, &wrong, &[]),
        get(port, &wrong, &[]),
        get(port, &wrong, &[])
    );
    let mut statuses: Vec<u16> = [&a, &b, &c].iter().map(|x| x.status).collect();
    statuses.sort();
    assert_eq!(statuses, [403, 403, 429]);
    let refused = [&a, &b, &c].into_iter().find(|x| x.status == 429).unwrap();
    let retry: u64 = refused.header("retry-after").unwrap().parse().unwrap();
    assert!(retry >= 1);
    assert_eq!(refused.header("set-cookie"), None);
}

#[tokio::test]
// Verifies: ADR-0037
async fn a_tab_that_answers_no_ping_is_closed_and_frees_its_slot_while_one_that_reads_stays() {
    let tree = Tree::new("pings");
    let running = start_on(
        tree.root(),
        Limits {
            tabs: 1,
            ping: Duration::from_millis(100),
            idle: Duration::from_millis(300),
            ..Limits::default()
        },
    )
    .await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    // A tab that reads answers the pings, its client sending the pongs, and stays well past idle.
    let until = std::time::Instant::now() + Duration::from_secs(1);
    let mut pinged = 0;
    while std::time::Instant::now() < until {
        if let Ok(Some(Ok(Message::Ping(_)))) =
            tokio::time::timeout(Duration::from_millis(50), tab.socket.next()).await
        {
            pinged += 1;
        }
    }
    assert!(pinged >= 3, "{pinged} pings");
    assert!(
        tab.invoke("read_window", json!({ "top": 0, "rows": 5 }))
            .await
            .is_ok()
    );
    assert_eq!(running.sessions(), 1);
    // The same tab reading nothing more answers no ping: it is closed, its session dropped, and
    // its slot taken by the next tab.
    sessions_become(&running, 0).await;
    let mut next = Tab::open(&running).await;
    assert!(
        next.invoke("read_window", json!({ "top": 0, "rows": 5 }))
            .await
            .is_ok()
    );
    drop(tab);
}

/// Writes the certificate and key of `made` where a self-signed one is kept, the key private.
fn keep(dir: &Path, made: &rcgen::CertifiedKey<rcgen::KeyPair>) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("cert.pem"), made.cert.pem()).unwrap();
    std::fs::write(dir.join("key.pem"), made.signing_key.serialize_pem()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("key.pem"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
}

#[tokio::test]
// Verifies: ADR-0038
async fn a_kept_pair_in_part_or_a_key_others_may_read_is_refused_saying_what_to_do() {
    let tree = Tree::new("kept-pair");
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    // What is refused below is kept as a whole and private here, and starts.
    let whole = tree.0.join("whole");
    keep(&whole, &made);
    assert!(
        start_tls(tree.root(), "127.0.0.1:0", None, &whole)
            .await
            .is_ok()
    );

    for lone in ["cert.pem", "key.pem"] {
        let dir = tree.0.join(format!("only-{lone}"));
        keep(&dir, &made);
        let other = if lone == "cert.pem" {
            "key.pem"
        } else {
            "cert.pem"
        };
        std::fs::remove_file(dir.join(other)).unwrap();
        let started = start_tls(tree.root(), "127.0.0.1:0", None, &dir).await;
        let error = started.err().unwrap();
        assert!(
            error.contains(&format!("{lone} is kept in"))
                && error.contains("remove cert.pem and key.pem"),
            "{error}"
        );
        // Nothing was made in its place.
        assert!(!dir.join(other).exists());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tree.0.join("readable");
        keep(&dir, &made);
        std::fs::set_permissions(dir.join("key.pem"), std::fs::Permissions::from_mode(0o640))
            .unwrap();
        let error = start_tls(tree.root(), "127.0.0.1:0", None, &dir)
            .await
            .err()
            .unwrap();
        assert!(error.contains("may be read by other accounts"), "{error}");
    }
}

#[tokio::test]
// Verifies: ADR-0038, Q-5
async fn on_an_unspecified_address_the_certificate_names_the_host_and_is_checked_for_it() {
    let tree = Tree::new("unspecified");
    let host = ocelli_server::tls::host_name().expect("the test machine's host name is a DNS name");
    // A certificate kept for another host name, as after the machine was renamed, is refused.
    let renamed = tree.0.join("renamed");
    keep(
        &renamed,
        &rcgen::generate_simple_self_signed(vec!["other-host.example".into()]).unwrap(),
    );
    let error = start_tls(tree.root(), "0.0.0.0:0", None, &renamed)
        .await
        .err()
        .unwrap();
    assert!(error.contains(&format!("is not for {host}")), "{error}");

    let keep_in = tree.0.join("config");
    let running = start_tls(tree.root(), "0.0.0.0:0", None, &keep_in)
        .await
        .unwrap();
    let port = running.port;
    assert!(
        running
            .url
            .starts_with(&format!("https://{host}:{port}/?token=")),
        "{}",
        running.url
    );
    let cert = pem_cert(&keep_in.join("cert.pem"));
    // Reached under the host name, it logs in.
    let target = running.url.split_once(&format!(":{port}")).unwrap().1;
    let stream = tls_connect(port, &cert, &host).await;
    let answer = get_over(stream, &format!("{host}:{port}"), target, &[]).await;
    assert_eq!(answer.status, 303);
    // An address the certificate does not carry is refused, as the README says.
    let stream = tls_connect(port, &cert, &host).await;
    let by_address = get_over(stream, &format!("127.0.0.1:{port}"), "/", &[]).await;
    assert_eq!(by_address.status, 403);
    assert_eq!(
        by_address.body,
        "This server answers only under a name its certificate carries"
    );
    // A second start reuses it, rechecked for the same host name.
    let again = start_tls(tree.root(), "0.0.0.0:0", None, &keep_in)
        .await
        .unwrap();
    assert_eq!(again.fingerprint, running.fingerprint);
}

#[tokio::test]
// Verifies: ADR-0038
async fn on_an_ipv6_address_the_certificate_names_it() {
    // Where the machine has no IPv6 loopback, there is nothing to listen on: the test says so.
    if std::net::TcpListener::bind("[::1]:0").is_err() {
        eprintln!("no IPv6 loopback on this machine: the IPv6 listener is not tested here");
        return;
    }
    let tree = Tree::new("ipv6");
    let keep_in = tree.0.join("config");
    let running = start_tls(tree.root(), "[::1]:0", None, &keep_in)
        .await
        .unwrap();
    let port = running.port;
    assert!(
        running
            .url
            .starts_with(&format!("https://[::1]:{port}/?token="))
    );
    let cert = pem_cert(&keep_in.join("cert.pem"));
    let target = running.url.split_once(&format!(":{port}")).unwrap().1;
    let stream = tls_connect_to(&format!("[::1]:{port}"), &cert, "::1").await;
    let answer = get_over(stream, &format!("[::1]:{port}"), target, &[]).await;
    assert_eq!(answer.status, 303);
}
