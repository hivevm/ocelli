//! Verifies: ADR-0035, Q-7
//!
//! A fetch sends a credential only over an encrypted connection to the host it belongs to. A
//! local server asks every request for credentials and records whether any came; the developer's
//! global configuration names a credential helper holding a password for another host, and the
//! repository's own names a helper of its own. libgit2 reads the global configuration from its
//! search path, which holds for the whole process: these tests run in a binary of their own.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod git_command;
use git_command::git_command;
use git2::ConfigLevel;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::io::{BufRead, BufReader, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

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

/// The developer's global configuration, once for the process: a helper that holds a password for
/// `example.invalid`, and one that leaves a marker where it ran.
fn developer(root: &Path) -> PathBuf {
    static ONCE: Once = Once::new();
    let global = root.join("global");
    let ran = root.join("ran");
    ONCE.call_once(|| {
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
    ran
}

/// A repository whose `origin` is `url`, its own configuration naming a helper that leaves a
/// marker in `ran`.
fn repository(dir: &Path, url: &str, ran: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let run = |args: &[&str]| {
        assert!(git_command(dir).args(args).status().unwrap().success());
    };
    run(&["init", "--quiet"]);
    run(&["remote", "add", "origin", url]);
    run(&[
        "config",
        "credential.helper",
        &format!("!touch {}", ran.display()),
    ]);
}

/// The credentials a request carried, decoded from its `Authorization: Basic` header. libgit2
/// sends a user name written into the address on its own, with an empty password: what the
/// address holds is the repository's, never the developer's.
fn credentials_sent(requests: &[String]) -> Vec<String> {
    requests
        .iter()
        .flat_map(|r| r.lines())
        .filter_map(|l| l.strip_prefix("Authorization: Basic "))
        .map(|b| String::from_utf8_lossy(&decode(b.trim())).into_owned())
        .collect()
}

fn decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
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

fn fetch(dir: &Path) -> String {
    let fetch = Write::Fetch {
        remote: "origin".into(),
    };
    GitWriter.write(dir, &fetch).unwrap_err().to_string()
}

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ocelli-credentials-{}-{name}", std::process::id()))
}

#[test]
fn a_line_break_in_the_address_hands_no_credential_over() {
    let (port, seen) = asking_server();
    let root = root("line-break");
    let developer_dir = std::env::temp_dir().join(format!("ocelli-dev-{}", std::process::id()));
    let ran = developer(&developer_dir);
    let url =
        format!("http://victim%0Ahost=example.invalid%0Aprotocol=https@127.0.0.1:{port}/repo.git");
    repository(&root, &url, &ran);
    let error = fetch(&root);
    assert!(error.contains("line break"), "{error}");
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty(), "the server was asked");
    let sent = credentials_sent(&seen);
    assert!(sent.iter().all(|c| !c.contains("SECRET")), "{sent:?}");
    assert!(!ran.exists(), "the repository's own helper ran");
}

#[test]
fn no_credential_goes_over_plain_http() {
    let (port, seen) = asking_server();
    let root = root("plain");
    let developer_dir = std::env::temp_dir().join(format!("ocelli-dev-{}", std::process::id()));
    let ran = developer(&developer_dir);
    repository(
        &root,
        &format!("http://victim@127.0.0.1:{port}/repo.git"),
        &ran,
    );
    let error = fetch(&root);
    assert!(error.contains("https only"), "{error}");
    let sent = credentials_sent(&seen.lock().unwrap());
    assert!(sent.iter().all(|c| !c.contains("SECRET")), "{sent:?}");
    assert!(!ran.exists(), "the repository's own helper ran");
}
