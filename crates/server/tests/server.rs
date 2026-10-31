//! The remote server tested where the browser meets it: over HTTP and the WebSocket, against a
//! server started in the test on a free port and a repository `git` made. Here are its reads, the
//! login and its throttle, its bounds, its confinement to the server root, and that it links no
//! part of Tauri or WebKit; its writes (`allow_writes.rs`, `writes.rs`, `transfers.rs`), the
//! shared list of commands (`command_list.rs`), TLS (`tls.rs`), and every command sent hostile
//! arguments (`hostile_commands.rs`) are tested in binaries of their own.

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
use ocelli_commands::Host;
use ocelli_core::port::Write;
use ocelli_server::{Limits, Running};
use serde_json::{Value, json};
use shown::stage_in;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Scratch, git, git_command, snapshot};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
// Verifies: ADR-0035, ADR-0036, ADR-0037, G-18
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

    // With the cookie the web interface is served, under the headers of ADR-0035 and ADR-0036.
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0037
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
// Verifies: ADR-0035
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
// Verifies: ADR-0037
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
// Verifies: ADR-0037, ADR-0035
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
// Verifies: ADR-0037
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036
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
// Verifies: ADR-0035
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
// Verifies: ADR-0037
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
// Verifies: ADR-0037
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
// Verifies: ADR-0037
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
        Err(ocelli_server::socket::scrub::READ_FAILED.to_string())
    );
}

#[tokio::test]
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0037
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
// Verifies: ADR-0037
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036, Q-5
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

#[tokio::test]
// Verifies: ADR-0037, ADR-0028
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
// Verifies: ADR-0037
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
// Verifies: ADR-0035, G-18
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
// Verifies: ADR-0035
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

#[tokio::test]
// Verifies: ADR-0035, ADR-0022, ADR-0040, G-18
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
    // one thing told otherwise, relative to the server root (ADR-0037).
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

#[tokio::test]
// Verifies: ADR-0037, ADR-0035
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

#[tokio::test]
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0035
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
