//! What the remote server's tests share: a server started on a free port, a browser tab on its
//! WebSocket that logs in as a browser does, and the shared `Host` as the desktop host holds it,
//! to compare the server's answers with. Each test binary of the server declares `support`
//! beside it.

// A test binary uses what it needs of the module and leaves the rest.
#![allow(dead_code)]

use crate::support::{Scratch, git};
use futures_util::{SinkExt, StreamExt};
use ocelli_commands::{Host, Report};
use ocelli_core::port::Write;
use ocelli_server::{Limits, Options, Running, start};
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
pub struct Tree(
    pub PathBuf,
    /// Held so that the tree is removed when the test ends.
    pub Scratch,
);

impl Tree {
    pub fn new(name: &str) -> Tree {
        let scratch = Scratch::new(name);
        for repo in ["root", "outside"] {
            three_commits(&scratch.0.join(repo));
        }
        Tree(scratch.0.clone(), scratch)
    }

    pub fn root(&self) -> PathBuf {
        self.0.join("root")
    }
}

/// A repository with three commits at `repo`.
pub fn three_commits(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "--quiet", "-b", "main"]);
    for n in 1..=3 {
        std::fs::write(repo.join("f"), format!("{n}\n")).unwrap();
        git(repo, &["add", "f"]);
        git(repo, &["commit", "--quiet", "-m", &format!("commit {n}")]);
    }
}

pub async fn start_on(root: PathBuf, limits: Limits) -> Running {
    start(Options {
        root,
        port: 0,
        limits,
        listen: None,
        allow_writes: false,
    })
    .await
    .unwrap()
}

pub async fn server(tree: &Tree) -> Running {
    start_on(tree.root(), Limits::default()).await
}

/// A server started with `--allow-writes` on `root` (ADR-0037).
pub async fn writing_on(root: PathBuf) -> Running {
    writing_within(root, Limits::default()).await
}

/// A server started with `--allow-writes` on `root`, within `limits`.
pub async fn writing_within(root: PathBuf, limits: Limits) -> Running {
    start(Options {
        root,
        port: 0,
        limits,
        listen: None,
        allow_writes: true,
    })
    .await
    .unwrap()
}

/// An answer over HTTP: its status, its headers with lowercase names, and its body.
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// `GET target` under the `Host` given, with the extra header lines.
pub async fn get_as(port: u16, host: &str, target: &str, extra: &[(&str, &str)]) -> Answer {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    get_over(stream, host, target, extra).await
}

/// `GET target` over `stream`, under the `Host` given, with the extra header lines.
pub async fn get_over<S>(mut stream: S, host: &str, target: &str, extra: &[(&str, &str)]) -> Answer
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut request = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    // What arrived before the connection closed is the answer, a TLS connection closed without
    // its last record included.
    let _ = stream.read_to_end(&mut raw).await;
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").expect("an answer");
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

pub async fn get(port: u16, target: &str, extra: &[(&str, &str)]) -> Answer {
    get_as(port, &format!("localhost:{port}"), target, extra).await
}

/// The path and query of the URL the server printed.
pub fn target(running: &Running) -> String {
    let prefix = format!("http://localhost:{}", running.port);
    running.url.strip_prefix(&prefix).unwrap().to_string()
}

/// The session cookie the printed URL gives, as a `Cookie` header carries it.
pub async fn log_in(running: &Running) -> String {
    let answer = get(running.port, &target(running), &[]).await;
    assert_eq!(answer.status, 303);
    let set = answer.header("set-cookie").unwrap();
    set.split(';').next().unwrap().to_string()
}

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A WebSocket handshake with the cookie and origin given; the status of a refusal.
pub async fn connect(port: u16, cookie: Option<&str>, origin: Option<&str>) -> Result<Socket, u16> {
    handshake(port, "/ws", cookie, origin).await
}

/// A WebSocket handshake to `target` with the cookie and origin given; the status of a refusal.
pub async fn handshake(
    port: u16,
    target: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<Socket, u16> {
    let mut request = format!("ws://localhost:{port}{target}")
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
pub struct Tab {
    pub socket: Socket,
    pub events: Vec<(String, Value)>,
    /// Answers that arrived while another command's was waited for.
    pub answers: Vec<Value>,
    pub next: u64,
}

impl Tab {
    pub async fn open(running: &Running) -> Tab {
        let cookie = log_in(running).await;
        let origin = format!("http://localhost:{}", running.port);
        let socket = connect(running.port, Some(&cookie), Some(&origin))
            .await
            .unwrap();
        Tab {
            socket,
            events: Vec::new(),
            answers: Vec::new(),
            next: 0,
        }
    }

    pub async fn message(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(20), self.socket.next())
                .await
                .expect("the server answers")
                .unwrap()
                .unwrap();
            match message {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                // A ping the server sends while a command runs is answered by the client, as a
                // browser answers it, and is no message of the command's.
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("not a text message: {other:?}"),
            }
        }
    }

    /// Sends a command as `invoke` would, without waiting for its answer; its id.
    pub async fn send(&mut self, cmd: &str, args: Value) -> u64 {
        self.next += 1;
        let command = json!({ "id": self.next, "cmd": cmd, "args": args });
        self.socket
            .send(Message::Text(command.to_string().into()))
            .await
            .unwrap();
        self.next
    }

    /// Sends a command as `invoke` would and returns its answer, keeping the events that
    /// arrive meanwhile.
    pub async fn invoke(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        let id = self.send(cmd, args).await;
        self.answer(id).await
    }

    /// The answer to the command sent with `id`, keeping the events that arrive meanwhile.
    pub async fn answer(&mut self, id: u64) -> Result<Value, String> {
        let message = loop {
            if let Some(at) = self.answers.iter().position(|m| m["id"] == id) {
                break self.answers.remove(at);
            }
            let message = self.message().await;
            self.keep(message);
        };
        match message.get("err") {
            Some(e) => Err(e.as_str().unwrap().to_string()),
            None => Ok(message["ok"].clone()),
        }
    }

    /// Keeps an event with the others, and an answer until its command's is asked for.
    pub fn keep(&mut self, message: Value) {
        match message["event"].as_str() {
            Some(event) => self
                .events
                .push((event.to_string(), message["payload"].clone())),
            None => {
                assert!(
                    message["id"].is_u64(),
                    "neither an event nor an answer: {message}"
                );
                self.answers.push(message);
            }
        }
    }

    /// Whether the command sent with `id` has been answered by now, among what has arrived.
    pub fn answered(&self, id: u64) -> bool {
        self.answers.iter().any(|m| m["id"] == id)
    }

    /// Waits for the event `name` whose payload `matches`.
    pub async fn until(&mut self, name: &str, matches: impl Fn(&Value) -> bool) -> Value {
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
pub struct Events(pub mpsc::UnboundedSender<(String, Value)>);

impl Report for Events {
    fn report<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self
            .0
            .send((event.to_string(), serde_json::to_value(payload).unwrap()));
    }
}

/// The window the shared commands, which the desktop host answers from, give for `repo` once its
/// whole history has loaded.
pub async fn desktop_window(repo: &Path, top: usize, rows: usize) -> Value {
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

/// The repository at `root` with changes to commit as Ada, whom its configuration names: a
/// changed file `f` and a new one, `new`.
pub fn with_changes(root: &Path) {
    git(root, &["config", "user.name", "Ada"]);
    git(root, &["config", "user.email", "ada@example.invalid"]);
    std::fs::write(root.join("f"), "changed\n").unwrap();
    std::fs::write(root.join("new"), "new\n").unwrap();
}

/// `write` performed in `tab` as the web interface performs it: asked first what it discards,
/// which is what the desktop host answers, and then performed without a confirmation.
pub async fn perform(tab: &mut Tab, write: &Write) -> Result<Value, String> {
    perform_as(tab, write, false).await
}

/// `write` performed in `tab` as the web interface performs it, asked first what it discards,
/// which is what the desktop host answers, and sent `confirmed` as the developer answered.
pub async fn perform_as(tab: &mut Tab, write: &Write, confirmed: bool) -> Result<Value, String> {
    assert_eq!(
        tab.invoke("write_discards", json!({ "write": write }))
            .await,
        Ok(to_value(Host::write_discards(write))),
        "{write:?}"
    );
    tab.invoke(
        "perform_write",
        json!({ "write": write, "confirmed": confirmed }),
    )
    .await
}

/// Opens `path` in `tab` and waits for its load to end; the refs it sent.
pub async fn open_and_load(tab: &mut Tab, path: &str) -> Value {
    tab.events.clear();
    assert_eq!(
        tab.invoke("open_repo", json!({ "path": path })).await,
        Ok(Value::Null),
        "{path}"
    );
    tab.until("history-grew", |p| p["loading"] == false).await;
    assert!(
        tab.events
            .contains(&("set-repo-label".to_string(), json!(path))),
        "{:?}",
        tab.events
    );
    tab.until("load-refs", |_| true).await
}

/// The shared commands as the desktop host holds them for its window, with the events they
/// reported, to compare the server's answers with.
pub struct Desktop {
    pub host: Host,
    pub events: mpsc::UnboundedReceiver<(String, Value)>,
    pub tx: mpsc::UnboundedSender<(String, Value)>,
}

impl Desktop {
    /// The desktop host's answers for `repo`, once its whole history has loaded; the refs it sent.
    pub async fn open(repo: &Path) -> (Desktop, Value) {
        let (tx, mut events) = mpsc::unbounded_channel();
        let host = Host::default();
        host.open_repo(repo.to_path_buf(), ".".into(), None, Events(tx.clone()))
            .unwrap();
        let mut refs = Value::Null;
        while let Some((event, payload)) = events.recv().await {
            if event == "load-refs" {
                refs = payload;
            } else if event == "history-grew" && payload["loading"] == false {
                break;
            }
        }
        (Desktop { host, events, tx }, refs)
    }

    /// The `commit-files` payload the desktop host sends for `sha`.
    pub async fn commit_files(&mut self, sha: &str) -> Value {
        self.host
            .select_commit(sha.to_string(), &Events(self.tx.clone()))
            .await
            .unwrap();
        loop {
            let (event, payload) = self.events.recv().await.unwrap();
            if event == "commit-files" {
                return payload;
            }
        }
    }
}

pub fn to_value(answer: impl Serialize) -> Value {
    serde_json::to_value(answer).unwrap()
}

impl Desktop {
    /// `write` performed as the desktop host's `perform_write` performs it, without a
    /// confirmation; its answer as the browser would read it.
    pub async fn perform(&self, write: &Write) -> Result<Value, String> {
        self.write(write, false).await
    }

    /// `write` performed as the desktop host's `perform_write` performs it, `confirmed` as the
    /// developer answered; its answer as the browser would read it, what git said replaced.
    pub async fn write(&self, write: &Write, confirmed: bool) -> Result<Value, String> {
        self.write_saying(write, confirmed)
            .await
            .map_err(ocelli_server::socket::scrub_write)
    }

    /// [`Desktop::write`], its failure as the desktop host reads it, with what git said.
    pub async fn write_saying(
        &self,
        write: &Write,
        confirmed: bool,
    ) -> Result<Value, ocelli_commands::HostError> {
        self.host
            .perform_write(write.clone(), confirmed, Events(self.tx.clone()))
            .await
            .map(to_value)
    }

    /// The row the load after a write selects, which the web interface starts with the commit
    /// the write made (ADR-0033).
    pub async fn reload(&mut self, repo: &Path, select: &str) -> Value {
        let to = Events(self.tx.clone());
        self.host
            .open_repo(repo.to_path_buf(), ".".into(), Some(select.into()), to)
            .unwrap();
        let selects = async {
            loop {
                let (event, payload) = self.events.recv().await.unwrap();
                if event == "history-grew" && !payload["select"].is_null() {
                    return payload["select"].clone();
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), selects)
            .await
            .expect("the reload selects the commit")
    }
}

/// A repository below a writing server's root, open in a tab, and its twin, open in the shared
/// `Host` as the desktop host holds it: each write made on both, from what each side shows, ends
/// alike on both (ADR-0037). The twin is copied with `cp`, as the writer's own tests, which run
/// on unix, copy theirs.
#[cfg(unix)]
pub struct Pair {
    pub root: PathBuf,
    pub other: PathBuf,
    pub running: Running,
    pub tab: Tab,
    pub desktop: Desktop,
    pub scratch: Scratch,
    pub twin: Option<Scratch>,
}

#[cfg(unix)]
impl Pair {
    /// The repository `setup` makes at `root` in a scratch directory named `name`, its twin, a
    /// server started with `--allow-writes` on `root`, a tab with the repository loaded, and the
    /// desktop's `Host` on the twin.
    pub async fn new(name: &str, setup: impl FnOnce(&Path)) -> Pair {
        Pair::served(name, |scratch| setup(&scratch.join("root")), false).await
    }

    /// [`Pair::new`], the server's root the scratch directory `root` lies in rather than `root`
    /// itself, where `setup`, given the scratch directory, may make more beside the repository,
    /// such as a remote below the server root. The twin is `desktop` beside `root`, below the
    /// same root: the confinement holds for the whole process, the desktop's `Host` of the test
    /// included, which then reaches what the server reaches.
    pub async fn beside(name: &str, setup: impl FnOnce(&Path)) -> Pair {
        Pair::served(name, setup, true).await
    }

    async fn served(name: &str, setup: impl FnOnce(&Path), from_scratch: bool) -> Pair {
        let scratch = Scratch::new(name);
        let root = scratch.0.join("root");
        std::fs::create_dir_all(&root).unwrap();
        setup(&scratch.0);
        let (twin, other) = if from_scratch {
            let other = scratch.0.join("desktop");
            let copied = std::process::Command::new("cp")
                .arg("-a")
                .arg(&root)
                .arg(&other)
                .status()
                .unwrap();
            assert!(copied.success());
            (None, other)
        } else {
            let twin = crate::support::twin(&scratch, &format!("{name}-desktop"));
            let other = twin.0.join("root");
            (Some(twin), other)
        };
        // The copy has new inodes: every file is read again rather than the index trusted.
        crate::support::git_output(&other, &["update-index", "-q", "--refresh"]);
        let (served, open) = if from_scratch {
            (scratch.0.clone(), "root")
        } else {
            (root.clone(), ".")
        };
        let running = writing_on(served).await;
        let mut tab = Tab::open(&running).await;
        open_and_load(&mut tab, open).await;
        let (desktop, _) = Desktop::open(&other).await;
        Pair {
            root,
            other,
            running,
            tab,
            desktop,
            scratch,
            twin,
        }
    }

    /// The write `make` makes from what each side shows, performed in the browser and by the
    /// desktop host, `confirmed` alike: the two answers are alike, a commit made by its content,
    /// and so are the twins. The server's answer.
    pub async fn alike(
        &mut self,
        make: impl Fn(&Path) -> Write,
        confirmed: bool,
    ) -> Result<Value, String> {
        let (write, mirrored) = (make(&self.root), make(&self.other));
        let here = perform_as(&mut self.tab, &write, confirmed).await;
        let there = self.desktop.write(&mirrored, confirmed).await;
        match (&here, &there) {
            (Ok(Value::String(_)), Ok(Value::String(_))) => {}
            _ => assert_eq!(here, there, "{write:?}: the server, then the desktop host"),
        }
        crate::support::assert_twins_alike(&self.root, &self.other, &format!("{write:?}"));
        here
    }

    /// The write `make` makes, which discards work (ADR-0034): refused on both sides without a
    /// confirmation, nothing changed, and then performed with it, as [`Pair::alike`] performs it.
    /// The server's answer to the confirmed write.
    pub async fn confirmed(&mut self, make: impl Fn(&Path) -> Write) -> Result<Value, String> {
        let write = make(&self.root);
        assert!(
            Host::write_discards(&write).is_some(),
            "{write:?} discards nothing"
        );
        self.refused(&make, false, "needs the developer's confirmation")
            .await;
        self.alike(make, true).await
    }

    /// The write `make` makes, refused on both sides, `confirmed` alike, with nothing changed on
    /// either: in the browser as `WRITE_FAILED`, by the desktop host for the reason `why`, so
    /// that the refusal is the one of the guard the test is about.
    pub async fn refused(&mut self, make: impl Fn(&Path) -> Write, confirmed: bool, why: &str) {
        let write = make(&self.root);
        let snapshot_of = crate::support::snapshot;
        let before = (snapshot_of(&self.root), snapshot_of(&self.other));
        assert_eq!(
            perform_as(&mut self.tab, &write, confirmed).await,
            Err(ocelli_server::socket::WRITE_FAILED.to_string()),
            "{write:?}"
        );
        let there = self
            .desktop
            .write_saying(&make(&self.other), confirmed)
            .await;
        assert!(
            matches!(&there, Err(ocelli_commands::HostError::Git(said)) if said.contains(why)),
            "{write:?}: the desktop host answers {there:?}"
        );
        assert_eq!(snapshot_of(&self.root), before.0, "{write:?}");
        assert_eq!(snapshot_of(&self.other), before.1, "{write:?}");
    }

    /// `change` made by hand on both sides, as `git` or an editor would make it.
    pub fn on_both(&self, change: impl Fn(&Path)) {
        change(&self.root);
        change(&self.other);
    }
}
