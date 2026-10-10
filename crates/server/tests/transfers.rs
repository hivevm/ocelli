//! Fetching, pulling, and pushing through the remote server (ADR-0037, ADR-0035): each made in a
//! tab of a server started with `--allow-writes` and by the shared `Host` as the desktop host
//! performs it, the two ending alike; a transfer's progress reaches the tab that started it, a
//! fetch and a pull's fetch stop on `cancel_write` (G-13), and a repository's own credential
//! helper does not run (Q-7).
//!
//! A remote on this machine is reached only where it lies below the server root, since libgit2
//! opens it in-process (ADR-0039, ADR-0037): the tests' remote lies below it, beside the
//! repository, and one outside is refused; a remote over the network is the server account's to
//! reach, with its own credentials. libgit2 reads the developer's global configuration from its search path, which holds
//! for the whole process: these tests run in a binary of their own, every test setting it first.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
// The twin is copied with `cp`, as the writer's own tests, which run on unix, copy theirs.
#![cfg(unix)]

mod browser;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Pair, Tab, Tree, open_and_load, rev, writing_on};
use git2::ConfigLevel;
use ocelli_core::port::Write;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;
use support::{Scratch, git, snapshot, write_file};

/// The developer's global configuration, once for the process: a credential helper that holds a
/// password for `example.invalid`. Every test of this binary calls it before libgit2 reads any
/// configuration.
fn developer() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let global =
            std::env::temp_dir().join(format!("ocelli-transfers-dev-{}", std::process::id()));
        std::fs::create_dir_all(&global).unwrap();
        let store = global.join("store");
        std::fs::write(&store, "https://victim:SECRET@example.invalid\n").unwrap();
        std::fs::write(
            global.join(".gitconfig"),
            format!(
                "[credential]\n\thelper = store --file {}\n",
                store.display()
            ),
        )
        .unwrap();
        for level in [ConfigLevel::Global, ConfigLevel::XDG, ConfigLevel::System] {
            // SAFETY: set once, before any test of this binary uses libgit2.
            #[allow(unsafe_code)]
            unsafe { git2::opts::set_search_path(level, &global) }.unwrap();
        }
    });
}

/// Commits a change to `file` in `dir` with `git`.
fn commit(dir: &Path, file: &str, message: &str) {
    write_file(dir, file, &format!("{message}\n"));
    git(dir, &["add", file]);
    git(dir, &["commit", "--quiet", "-m", message]);
}

/// A bare remote at `remote`, with `main` and `follow`, and a clone of it in `seed` that the
/// tests push the remote's further history from, by Ada.
fn remote_with_seed(remote: &Path, seed: &Path) {
    git(
        remote.parent().unwrap(),
        &["init", "--quiet", "--bare", "-b", "main", "remote.git"],
    );
    git(
        seed.parent().unwrap(),
        &["clone", "--quiet", remote.to_str().unwrap(), "seed"],
    );
    git(seed, &["config", "user.name", "Ada"]);
    git(seed, &["config", "user.email", "ada@example.invalid"]);
    commit(seed, "f", "one");
    commit(seed, "f", "two");
    git(seed, &["push", "--quiet", "origin", "main"]);
    git(seed, &["push", "--quiet", "origin", "main:follow"]);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0035, G-8, G-7, G-13
async fn fetching_pulling_and_pushing_in_the_browser_end_as_the_desktop_leaves_them() {
    developer();
    // The seed is the test's own `git`, wherever it lies; the remote lies below the server root,
    // beside the repository, where the remote server reaches it.
    let seeding = Scratch::new("transfers-seed");
    let seed = seeding.0.join("seed");
    let mut remote = PathBuf::new();
    let mut pair = Pair::beside("transfers", |scratch| {
        remote = scratch.join("remote.git");
        remote_with_seed(&remote, &seed);
        let root = &scratch.join("root");
        std::fs::remove_dir(root).unwrap();
        git(
            scratch,
            &["clone", "--quiet", remote.to_str().unwrap(), "root"],
        );
        git(root, &["config", "user.name", "Ada"]);
        git(root, &["config", "user.email", "ada@example.invalid"]);
        git(
            root,
            &["branch", "--quiet", "--track", "follow", "origin/follow"],
        );
        // main ahead of the remote by one commit, and the remote ahead on follow and a new
        // branch, so that each transfer has something to do.
        commit(root, "mine", "mine");
        git(
            &seed,
            &["switch", "--quiet", "-c", "follow", "origin/follow"],
        );
        commit(&seed, "g", "upstream");
        git(
            &seed,
            &["push", "--quiet", "origin", "follow", "follow:other"],
        );
    })
    .await;
    let done = |answer: Result<Value, String>| assert_eq!(answer, Ok(Value::Null));
    // A second tab on the repository, which every write tells to load again.
    let mut beside = Tab::open(&pair.running).await;
    open_and_load(&mut beside, "root").await;

    // The fetch, its progress reaching the tab that started it.
    pair.tab.events.clear();
    done(
        pair.alike(
            |_| Write::Fetch {
                remote: "origin".into(),
            },
            false,
        )
        .await,
    );
    let progress: Vec<&Value> = pair
        .tab
        .events
        .iter()
        .filter(|(event, _)| event == "write-progress")
        .map(|(_, payload)| payload)
        .collect();
    assert!(!progress.is_empty(), "the fetch reported its progress");
    assert!(
        progress
            .iter()
            .all(|p| p.as_str().unwrap().starts_with("Fetching origin")),
        "{progress:?}"
    );
    beside.until("repo-written", |_| true).await;
    assert_eq!(rev(&pair.root, "origin/other"), rev(&seed, "follow"));

    // The push of main, which sets nothing new: main tracks origin/main already.
    done(
        pair.alike(
            |d| Write::Push {
                branch: "main".into(),
                tip: rev(d, "main"),
            },
            false,
        )
        .await,
    );
    assert_eq!(rev(&remote, "main"), rev(&pair.root, "main"));

    // The pull of follow, fast-forward.
    done(
        pair.alike(
            |d| Write::SwitchBranch {
                name: "follow".into(),
                tip: rev(d, "follow"),
            },
            false,
        )
        .await,
    );
    done(
        pair.alike(
            |d| Write::Pull {
                branch: "follow".into(),
                tip: rev(d, "follow"),
            },
            false,
        )
        .await,
    );
    assert_eq!(rev(&pair.root, "follow"), rev(&seed, "follow"));

    // A pull of a branch that has diverged is refused after its fetch moved the remote branch,
    // and the other tab is told to load again all the same (ADR-0035, ADR-0037).
    commit(&seed, "g", "upstream again");
    git(&seed, &["push", "--quiet", "origin", "follow"]);
    pair.on_both(|d| commit(d, "h", "mine on follow"));
    beside.events.clear();
    let moved = rev(&seed, "follow");
    assert_eq!(
        pair.alike(
            |d| Write::Pull {
                branch: "follow".into(),
                tip: rev(d, "follow"),
            },
            false,
        )
        .await,
        Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string())
    );
    assert_eq!(rev(&pair.root, "origin/follow"), moved, "the fetch is kept");
    beside.until("repo-written", |_| true).await;
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0039, Q-5
async fn a_remote_on_this_machine_outside_the_root_is_neither_fetched_nor_pushed() {
    developer();
    let tree = Tree::new("transfers-outside");
    let root = tree.root();
    // A remote outside the root, ahead of the repository, by its path and by a file:// address.
    let outside = tree.0.join("outside.git");
    git(
        &tree.0,
        &["clone", "--quiet", "--bare", "outside", "outside.git"],
    );
    let (by_path, by_url) = (
        outside.to_str().unwrap().to_string(),
        format!("file://{}", outside.display()),
    );
    git(&root, &["remote", "add", "path", &by_path]);
    git(&root, &["remote", "add", "url", &by_url]);
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let (before, remote) = (snapshot(&root), git(&outside, &["for-each-ref"]));
    let failed = Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string());
    for name in ["path", "url"] {
        let fetch = Write::Fetch {
            remote: name.into(),
        };
        assert_eq!(browser::perform(&mut tab, &fetch).await, failed, "{name}");
        git(&root, &["config", "branch.main.pushRemote", name]);
        let push = Write::Push {
            branch: "main".into(),
            tip: rev(&root, "HEAD"),
        };
        let before = snapshot(&root);
        assert_eq!(browser::perform(&mut tab, &push).await, failed, "{name}");
        assert_eq!(snapshot(&root), before, "{name}");
    }
    git(&root, &["config", "--unset", "branch.main.pushRemote"]);
    assert_eq!(snapshot(&root), before);
    assert_eq!(
        git(&outside, &["for-each-ref"]),
        remote,
        "the remote is left alone"
    );
    // `git`, which the server's confinement does not bind, fetches from it: there was a fetch
    // to make.
    git(&root, &["fetch", "--quiet", "path"]);
    assert_ne!(snapshot(&root), before, "a fetch was there to make");
}

/// A server that speaks git's smart HTTP protocol far enough for a fetch to reach the pack: it
/// advertises `main` at a commit nobody has, and answers the request for it with git's
/// acknowledgement and then, every few milliseconds, a line of progress, never the pack, until
/// the connection closes. Without a stop the fetch runs until libgit2 gives the server up;
/// `closed` is set once the client has closed the connection.
fn preparing_server() -> (u16, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let closed = Arc::new(AtomicBool::new(false));
    let told = Arc::clone(&closed);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let told = Arc::clone(&told);
            std::thread::spawn(move || answer_git(stream, &told));
        }
    });
    (port, closed)
}

/// One line of git's packet protocol.
fn pkt(line: &[u8]) -> Vec<u8> {
    let mut out = format!("{:04x}", line.len() + 4).into_bytes();
    out.extend_from_slice(line);
    out
}

/// A chunk of an HTTP body sent in chunks.
fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

fn answer_git(stream: TcpStream, closed: &AtomicBool) {
    let mut reader = BufReader::new(&stream);
    let (mut request, mut length, mut chunked) = (String::new(), 0, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(n) = lower.strip_prefix("content-length:") {
            length = n.trim().parse().unwrap_or(0);
        }
        chunked |= lower.starts_with("transfer-encoding:") && lower.contains("chunked");
        if line == "\r\n" {
            break;
        }
        request.push_str(&line);
    }
    // The request's body, the wants and haves, is read and not looked at.
    if chunked {
        loop {
            let mut size = String::new();
            if reader.read_line(&mut size).unwrap_or(0) == 0 {
                return;
            }
            let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            let mut body = vec![0; size + 2];
            if reader.read_exact(&mut body).is_err() || size == 0 {
                break;
            }
        }
    } else {
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
    }
    let mut out = &stream;
    let oid = "1".repeat(40);
    if request.starts_with("GET") {
        let mut body = pkt(b"# service=git-upload-pack\n");
        body.extend_from_slice(b"0000");
        body.extend(pkt(format!(
            "{oid} HEAD\0side-band-64k ofs-delta agent=git/fake\n"
        )
        .as_bytes()));
        body.extend(pkt(format!("{oid} refs/heads/main\n").as_bytes()));
        body.extend_from_slice(b"0000");
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = out.write_all(head.as_bytes());
        let _ = out.write_all(&body);
        return;
    }
    let head = "HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-result\r\n\
                Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if out.write_all(head.as_bytes()).is_err() || out.write_all(&chunk(&pkt(b"NAK\n"))).is_err() {
        return;
    }
    // Progress for at most a minute, longer than any test waits.
    for _ in 0..1200 {
        let line = pkt(b"\x02Counting objects: preparing\n");
        if out
            .write_all(&chunk(&line))
            .and_then(|()| out.flush())
            .is_err()
        {
            closed.store(true, Ordering::SeqCst);
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until `tab` has been told of `n` steps of its write's progress, the write still
/// running: its answer has not arrived.
async fn progress(tab: &mut Tab, id: u64, n: usize) {
    loop {
        let told = tab
            .events
            .iter()
            .filter(|(event, _)| event == "write-progress")
            .count();
        assert!(!tab.answered(id), "the write ended before it was stopped");
        if told >= n {
            return;
        }
        let message = tab.message().await;
        tab.keep(message);
    }
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0035, G-13
async fn a_fetch_and_a_pull_s_fetch_stop_when_the_browser_cancels_them() {
    developer();
    let (port, closed) = preparing_server();
    let tree = Tree::new("transfers-cancel");
    let root = tree.root();
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("http://127.0.0.1:{port}/r.git"),
        ],
    );
    // main tracks origin/main, which stands where main does.
    git(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        &root,
        &["branch", "--quiet", "--set-upstream-to", "origin/main"],
    );
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let head = rev(&root, "HEAD");
    let writes = [
        Write::Fetch {
            remote: "origin".into(),
        },
        Write::Pull {
            branch: "main".into(),
            tip: head,
        },
    ];
    for write in writes {
        closed.store(false, Ordering::SeqCst);
        tab.events.clear();
        let id = tab
            .send(
                "perform_write",
                json!({ "write": write, "confirmed": false }),
            )
            .await;
        // The server keeps preparing: the write runs on, reporting, until it is stopped.
        progress(&mut tab, id, 2).await;
        assert_eq!(tab.invoke("cancel_write", json!({})).await, Ok(Value::Null));
        assert_eq!(
            tab.answer(id).await,
            Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string()),
            "{write:?}"
        );
        let since = std::time::Instant::now();
        while !closed.load(Ordering::SeqCst) {
            assert!(
                since.elapsed() < Duration::from_secs(20),
                "{write:?}: the connection stayed open"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(snapshot(&root), before, "{write:?}");
    }
}

#[tokio::test]
// Verifies: ADR-0037, G-13
async fn a_tab_closed_while_it_fetches_stops_the_fetch_and_gives_the_repository_s_turn_back() {
    developer();
    let (port, closed) = preparing_server();
    let tree = Tree::new("transfers-closed");
    let root = tree.root();
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("http://127.0.0.1:{port}/r.git"),
        ],
    );
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let fetch = Write::Fetch {
        remote: "origin".into(),
    };
    let id = tab
        .send(
            "perform_write",
            json!({ "write": fetch, "confirmed": false }),
        )
        .await;
    progress(&mut tab, id, 2).await;
    drop(tab);
    // The fetch stops: the connection to the remote closes, and the turn is free again.
    let since = std::time::Instant::now();
    while !closed.load(Ordering::SeqCst) {
        assert!(
            since.elapsed() < Duration::from_secs(20),
            "the fetch of a closed tab runs on"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (turns, common) = (
        running.turns().unwrap().clone(),
        ocelli_git::refs::common_dir(&root).unwrap(),
    );
    let taken = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::task::spawn_blocking(move || {
            turns
                .take(common, &AtomicBool::new(false))
                .map(drop)
                .is_ok()
        }),
    )
    .await
    .expect("the repository's turn is given back")
    .unwrap();
    assert!(taken);
    assert_eq!(snapshot(&root), before);
}

/// A server that answers every request with 401 and keeps each request's headers.
fn asking_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            kept.lock().unwrap().push(request);
            let _ = (&stream).write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"r\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (port, seen)
}

/// The bytes of base64 `text`, as an `Authorization: Basic` header carries them.
fn decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bits, mut n, mut out) = (0u32, 0, Vec::new());
    for c in text.bytes().filter(|&c| c != b'=') {
        let Some(v) = TABLE.iter().position(|&t| t == c) else {
            continue;
        };
        bits = bits << 6 | v as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
}

#[tokio::test]
// Verifies: ADR-0037, ADR-0035, Q-7
async fn a_fetch_from_the_browser_runs_no_credential_helper_of_the_repository_and_sends_no_secret()
{
    developer();
    let (port, seen) = asking_server();
    let tree = Tree::new("transfers-credentials");
    let root = tree.root();
    // Outside the server root, so that the marker is none of its files.
    let ran: PathBuf = tree.0.join("ran");
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("http://victim@127.0.0.1:{port}/repo.git"),
        ],
    );
    git(
        &root,
        &[
            "config",
            "credential.helper",
            &format!("!touch {}", ran.display()),
        ],
    );
    let running = writing_on(root.clone()).await;
    let mut tab = Tab::open(&running).await;
    open_and_load(&mut tab, ".").await;
    let before = snapshot(&root);
    let fetch = Write::Fetch {
        remote: "origin".into(),
    };
    assert_eq!(
        browser::perform(&mut tab, &fetch).await,
        Err(ocelli_server::socket::scrub::WRITE_FAILED.to_string())
    );
    let seen = seen.lock().unwrap();
    assert!(
        !seen.is_empty(),
        "the server was asked, and asked for credentials"
    );
    let sent: Vec<String> = seen
        .iter()
        .flat_map(|r| r.lines())
        .filter_map(|l| l.strip_prefix("Authorization: Basic "))
        .map(|b| String::from_utf8_lossy(&decode(b.trim())).into_owned())
        .collect();
    // libgit2 sends the user name the address carries, with no password, on its own; no
    // credential of the developer's helper, which holds one for another host only.
    assert!(sent.iter().all(|c| !c.contains("SECRET")), "{sent:?}");
    assert!(!ran.exists(), "the repository's own helper ran");
    assert_eq!(snapshot(&root), before);
}
