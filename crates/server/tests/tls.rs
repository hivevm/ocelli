//! Where the remote server listens and how it is reached there (ADR-0036): by default on the
//! loopback addresses alone over plain HTTP; with `--listen`, over TLS alone, with the developer's
//! certificate and key or a self-signed pair it makes and keeps for the developer, and a cookie
//! only a secure connection carries.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod browser;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Answer, Tree, get_over, server};
use ocelli_server::{Limits, Options, Running, start};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036
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
// Verifies: ADR-0036
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
// Verifies: ADR-0036, Q-5
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
// Verifies: ADR-0036
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
