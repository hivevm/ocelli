//! The remote server tested where the browser meets it: over HTTP and the WebSocket, against a
//! server started in the test on a free port and a repository `git` made.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

#[path = "../../git/tests/git_command/mod.rs"]
mod git_command;

use futures_util::{SinkExt, StreamExt};
use git_command::git_command;
use ocelli_commands::{Host, Report};
use ocelli_server::{Options, Running, start};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

/// A scratch tree, named for the test and the process and removed when the test ends: `root/`, a
/// repository with three commits, and `outside/`, a repository beside it.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Tree {
        let dir = std::env::temp_dir().join(format!("ocelli-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for repo in ["root", "outside"] {
            let repo = dir.join(repo);
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "--quiet", "-b", "main"]);
            for n in 1..=3 {
                std::fs::write(repo.join("f"), format!("{n}\n")).unwrap();
                git(&repo, &["add", "f"]);
                git(&repo, &["commit", "--quiet", "-m", &format!("commit {n}")]);
            }
        }
        Tree(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("root")
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir)
        .args([
            "-c",
            "user.name=Ada",
            "-c",
            "user.email=ada@example.invalid",
        ])
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

async fn server(tree: &Tree) -> Running {
    start(Options {
        root: tree.root(),
        port: 0,
    })
    .await
    .unwrap()
}

/// An answer over HTTP: its status, its headers with lowercase names, and its body.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// `GET target` under the `Host` given, with the extra header lines.
async fn get_as(port: u16, host: &str, target: &str, extra: &[(&str, &str)]) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Answer {
        status,
        headers,
        body: body.to_string(),
    }
}

async fn get(port: u16, target: &str, extra: &[(&str, &str)]) -> Answer {
    get_as(port, &format!("localhost:{port}"), target, extra).await
}

/// The path and query of the URL the server printed.
fn target(running: &Running) -> String {
    let prefix = format!("http://localhost:{}", running.port);
    running.url.strip_prefix(&prefix).unwrap().to_string()
}

/// The session cookie the printed URL gives, as a `Cookie` header carries it.
async fn log_in(running: &Running) -> String {
    let answer = get(running.port, &target(running), &[]).await;
    assert_eq!(answer.status, 303);
    let set = answer.header("set-cookie").unwrap();
    set.split(';').next().unwrap().to_string()
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A WebSocket handshake with the cookie and origin given; the status of a refusal.
async fn connect(port: u16, cookie: Option<&str>, origin: Option<&str>) -> Result<Socket, u16> {
    let mut request = format!("ws://localhost:{port}/ws")
        .into_client_request()
        .unwrap();
    for (name, value) in [("cookie", cookie), ("origin", origin)] {
        if let Some(value) = value {
            request
                .headers_mut()
                .insert(name, HeaderValue::from_str(value).unwrap());
        }
    }
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    match tokio_tungstenite::client_async(request, MaybeTlsStream::Plain(stream)).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(e) => panic!("the handshake failed otherwise: {e}"),
    }
}

/// A browser tab on the server: its socket, the events it has received, and the next id.
struct Tab {
    socket: Socket,
    events: Vec<(String, Value)>,
    next: u64,
}

impl Tab {
    async fn open(running: &Running) -> Tab {
        let cookie = log_in(running).await;
        let origin = format!("http://localhost:{}", running.port);
        let socket = connect(running.port, Some(&cookie), Some(&origin))
            .await
            .unwrap();
        Tab {
            socket,
            events: Vec::new(),
            next: 0,
        }
    }

    async fn message(&mut self) -> Value {
        let message = tokio::time::timeout(Duration::from_secs(20), self.socket.next())
            .await
            .expect("the server answers")
            .unwrap()
            .unwrap();
        let Message::Text(text) = message else {
            panic!("not a text message: {message:?}")
        };
        serde_json::from_str(&text).unwrap()
    }

    /// Sends a command as `invoke` would and returns its answer, keeping the events that
    /// arrive meanwhile.
    async fn invoke(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let command = json!({ "id": id, "cmd": cmd, "args": args });
        self.socket
            .send(Message::Text(command.to_string().into()))
            .await
            .unwrap();
        loop {
            let message = self.message().await;
            if message["id"] == id {
                return match message.get("err") {
                    Some(e) => Err(e.as_str().unwrap().to_string()),
                    None => Ok(message["ok"].clone()),
                };
            }
            self.keep(message);
        }
    }

    fn keep(&mut self, message: Value) {
        let event = message["event"].as_str().expect("an event").to_string();
        self.events.push((event, message["payload"].clone()));
    }

    /// Waits for the event `name` whose payload `matches`.
    async fn until(&mut self, name: &str, matches: impl Fn(&Value) -> bool) -> Value {
        let found = |events: &[(String, Value)]| {
            events
                .iter()
                .find(|(n, p)| n == name && matches(p))
                .map(|(_, p)| p.clone())
        };
        loop {
            if let Some(payload) = found(&self.events) {
                return payload;
            }
            let message = self.message().await;
            self.keep(message);
        }
    }
}

/// The events of the shared `Host`, for the answer the desktop host gives to compare with.
#[derive(Clone)]
struct Events(mpsc::UnboundedSender<(String, Value)>);

impl Report for Events {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self
            .0
            .send((event.to_string(), serde_json::to_value(payload).unwrap()));
    }
}

/// The window the shared commands, which the desktop host answers from, give for `repo` once its
/// whole history has loaded.
async fn desktop_window(repo: &Path, top: usize, rows: usize) -> Value {
    let host = Host::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    host.open_repo(repo.to_path_buf(), ".".into(), None, Events(tx))
        .unwrap();
    while let Some((event, payload)) = rx.recv().await {
        if event == "history-grew" && payload["loading"] == false {
            break;
        }
    }
    serde_json::to_value(host.read_window(top, rows)).unwrap()
}

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
            Err(format!("{path} is no repository below the server root"))
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
// Verifies: ADR-0037
async fn a_write_is_refused_while_the_server_only_reads() {
    let tree = Tree::new("write");
    let running = server(&tree).await;
    let mut tab = Tab::open(&running).await;
    tab.invoke("open_repo", json!({ "path": "." }))
        .await
        .unwrap();
    tab.until("history-grew", |p| p["loading"] == false).await;
    let head = git(&tree.root(), &["rev-parse", "HEAD"]);
    let write = json!({ "kind": "createTag", "name": "new", "commit": head });
    assert_eq!(
        tab.invoke(
            "perform_write",
            json!({ "write": write, "confirmed": false })
        )
        .await,
        Err(ocelli_server::socket::WRITES_REFUSED.to_string())
    );
    assert_eq!(git(&tree.root(), &["tag", "--list", "new"]), "");
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
