//! The remote server (ADR-0035): it serves the web interface to a browser and carries the desktop
//! host's commands and events over one WebSocket per tab, answering them through the commands
//! both hosts share. It listens on the machine's loopback addresses, or over TLS alone on an
//! address the developer names, and admits a browser only by
//! the connection token it prints, exchanged for a session cookie, from its own origin (ADR-0036),
//! and opens only the repositories below its server root (ADR-0037).

pub mod auth;
pub mod root;
pub mod socket;
pub mod tls;
pub mod turns;
pub mod web;

use auth::Gate;
use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequestParts, Request, State, ws::WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use root::Root;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

/// What the server is started with.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    /// The server root: required, there is no default (ADR-0037).
    pub root: PathBuf,
    /// The port on the loopback addresses; 0 lets the system choose a free one.
    pub port: u16,
    pub limits: Limits,
    /// The address named with `--listen`, served over TLS in place of the loopback addresses.
    pub listen: Option<Listen>,
    /// Whether the browser may write, as the desktop host writes, as the user the server runs as;
    /// without `--allow-writes` every write is refused (ADR-0035).
    pub allow_writes: bool,
}

/// What bounds the resources one client can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a connection may take to send a request's headers before it is closed.
    pub header_read: Duration,
    /// The most session ids kept; the oldest gives way to a new one.
    pub sessions: usize,
    /// The most messages queued for a tab; a tab that does not read them is closed.
    pub queue: usize,
    /// How long one message may take to be written to a tab before it is closed.
    pub send_wait: Duration,
    /// The most connections served at once; one past them is closed unanswered.
    pub connections: usize,
    /// The most tabs open at once; a handshake past them is refused.
    pub tabs: usize,
    /// The most entries of a repository's git directory checked for links when it opens.
    pub walk: usize,
    /// The most wrong tokens waiting for their answer at once; one past them is refused at once.
    pub waiting: usize,
    /// How often a tab is sent a ping.
    pub ping: Duration,
    /// How long a tab may send nothing, not even the answer to a ping, before it is closed.
    pub idle: Duration,
    /// The most writes that wait for one repository's turn; one past them is refused at once.
    pub writes_waiting: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            header_read: Duration::from_secs(10),
            sessions: 64,
            queue: 1024,
            send_wait: Duration::from_secs(30),
            connections: 128,
            tabs: 32,
            walk: ocelli_git::confine::WALK_CAP,
            waiting: 8,
            ping: Duration::from_secs(30),
            idle: Duration::from_secs(75),
            writes_waiting: 8,
        }
    }
}

/// An address the developer names, always served over TLS (ADR-0036).
#[derive(Debug, PartialEq, Eq)]
pub struct Listen {
    pub address: SocketAddr,
    /// The developer's certificate and key, in PEM files; without them a self-signed certificate
    /// is made on the first start and kept.
    pub cert: Option<(PathBuf, PathBuf)>,
    /// Where a self-signed certificate is kept; none for the user's configuration directory.
    pub keep_in: Option<PathBuf>,
}

pub const USAGE: &str = "Usage: ocelli-server --root <folder> [--port <port>] \
                         [--listen <address>[:<port>] [--cert <file> --key <file>]] \
                         [--allow-writes]";

impl Options {
    /// The options of a command line, the program's name first.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
        let mut args = args.into_iter().skip(1);
        let (mut root, mut port, mut listen, mut cert, mut key) = (None, 0, None, None, None);
        let mut allow_writes = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" => root = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
                "--port" => {
                    port = args
                        .next()
                        .and_then(|p| p.parse().ok())
                        .ok_or("--port takes a number from 0 to 65535")?
                }
                "--listen" => listen = Some(args.next().ok_or(USAGE)?),
                "--cert" => cert = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
                "--key" => key = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
                "--allow-writes" => allow_writes = true,
                _ => return Err(format!("Unknown argument {arg}\n{USAGE}")),
            }
        }
        let cert = match (cert, key) {
            (Some(cert), Some(key)) => Some((cert, key)),
            (None, None) => None,
            _ => return Err("--cert and --key are given together".into()),
        };
        let listen = match listen {
            Some(address) => Some(Listen {
                address: address
                    .parse()
                    .or_else(|_| {
                        let ip = address
                            .strip_prefix('[')
                            .and_then(|a| a.strip_suffix(']'))
                            .unwrap_or(&address);
                        ip.parse().map(|ip| SocketAddr::new(ip, port))
                    })
                    .map_err(|_| {
                        format!("--listen takes an address, with a port or with --port: {address}")
                    })?,
                cert,
                keep_in: None,
            }),
            None if cert.is_some() => return Err("--cert and --key go with --listen".into()),
            None => None,
        };
        Ok(Options {
            root: root.ok_or(USAGE)?,
            port,
            limits: Limits::default(),
            listen,
            allow_writes,
        })
    }
}

/// What every request is answered from.
struct Server {
    root: Arc<Root>,
    gate: Gate,
    /// Held while a token is checked, so that the wait after wrong ones holds for the whole
    /// server and not per connection (ADR-0036).
    token_turn: tokio::sync::Mutex<()>,
    /// A permit per wrong token that waits for its answer.
    waiting: Arc<Semaphore>,
    limits: Limits,
    /// A permit per open tab.
    tabs: Arc<Semaphore>,
    /// The tabs' load sessions the server holds.
    sessions: socket::Sessions,
    /// The turns of the repositories' writes, where the server was started with `--allow-writes`;
    /// none where it refuses every write (ADR-0035).
    writes: Option<turns::Turns>,
    /// The hashes of the inline scripts of the web interface's `index.html`.
    script_hashes: Vec<String>,
    /// The certificate of a server listening on a named address, none on the loopback addresses.
    tls: Option<Arc<tls::Tls>>,
}

/// A server that runs: the port it listens on and the URL that carries its connection token.
pub struct Running {
    pub port: u16,
    pub url: String,
    pub root: PathBuf,
    /// The addresses it listens on.
    pub addresses: Vec<SocketAddr>,
    /// The SHA-256 fingerprint of its certificate, where it speaks TLS (ADR-0036).
    pub fingerprint: Option<String>,
    sessions: socket::Sessions,
    writes: Option<turns::Turns>,
}

impl Running {
    /// How many tabs' load sessions the server holds now: one per open connection, and one a
    /// closed connection's commands or load still hold until they stop (ADR-0035).
    pub fn sessions(&self) -> usize {
        self.sessions.count()
    }

    /// The turns its tabs' writes take, where it was started with `--allow-writes` (ADR-0035).
    pub fn turns(&self) -> Option<&turns::Turns> {
        self.writes.as_ref()
    }
}

/// Starts the server (ADR-0036): by default on the loopback addresses over plain HTTP, IPv4
/// always, IPv6 where the machine has it, on the same port; with `--listen` on that address
/// alone, over TLS alone.
pub async fn start(options: Options) -> Result<Running, String> {
    let root = Root::new(&options.root)?.with_walk_cap(options.limits.walk);
    let tls = match &options.listen {
        None => None,
        Some(Listen {
            cert: Some((cert, key)),
            ..
        }) => Some(tls::Tls::from_files(cert, key)?),
        Some(Listen {
            address, keep_in, ..
        }) => {
            let dir = keep_in.clone().or_else(tls::default_dir).ok_or(
                "No configuration directory to keep a certificate in: give --cert and --key",
            )?;
            Some(tls::Tls::self_signed(&dir, address.ip())?)
        }
    };
    // Every repository the git adapter opens in this process is confined to the root: none is
    // found by searching upwards (ADR-0037).
    ocelli_git::confine::confine_to(root.path());
    let script_hashes = web::file("index.html")
        .map(|html| web::inline_script_hashes(&String::from_utf8_lossy(html)))
        .unwrap_or_default();
    let gate = Gate::new(options.limits.sessions);
    let server = Arc::new(Server {
        root: Arc::new(root),
        gate: if tls.is_some() { gate.over_tls() } else { gate },
        token_turn: tokio::sync::Mutex::new(()),
        waiting: Arc::new(Semaphore::new(options.limits.waiting)),
        tabs: Arc::new(Semaphore::new(options.limits.tabs)),
        sessions: socket::Sessions::default(),
        writes: options
            .allow_writes
            .then(|| turns::Turns::new(options.limits.writes_waiting)),
        limits: options.limits,
        script_hashes,
        tls: tls.map(Arc::new),
    });
    let listeners = match &options.listen {
        Some(listen) => vec![
            TcpListener::bind(listen.address)
                .await
                .map_err(|e| format!("Cannot listen on {}: {e}", listen.address))?,
        ],
        None => {
            let v4 = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, options.port)))
                .await
                .map_err(|e| format!("Cannot listen on 127.0.0.1:{}: {e}", options.port))?;
            let port = v4.local_addr().map_err(|e| e.to_string())?.port();
            let v6 = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port)))
                .await
                .ok();
            std::iter::once(v4).chain(v6).collect()
        }
    };
    let addresses = listeners
        .iter()
        .map(|l| l.local_addr().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let port = addresses[0].port();
    let app = Router::new()
        .fallback(handle)
        .with_state(Arc::clone(&server));
    // One count of connections for every listener.
    let connections = Arc::new(Semaphore::new(options.limits.connections));
    let acceptor = server.tls.as_ref().map(|tls| tls.acceptor.clone());
    for listener in listeners {
        tokio::spawn(accept(
            listener,
            app.clone(),
            acceptor.clone(),
            options.limits.header_read,
            Arc::clone(&connections),
        ));
    }
    let token = server.gate.token();
    let url = match &options.listen {
        None => format!("http://localhost:{port}/?token={token}"),
        Some(listen) => {
            let ip = listen.address.ip();
            let host = match ip {
                // An address that names every interface names none a browser reaches: the
                // host name the certificate carries stands in for it.
                _ if ip.is_unspecified() => gethostname::gethostname().to_string_lossy().into(),
                std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                std::net::IpAddr::V4(v4) => v4.to_string(),
            };
            format!("https://{host}:{port}/?token={token}")
        }
    };
    Ok(Running {
        port,
        url,
        root: server.root.path().to_path_buf(),
        addresses,
        fingerprint: server.tls.as_ref().map(|tls| tls.fingerprint()),
        sessions: server.sessions.clone(),
        writes: server.writes.clone(),
    })
}

/// Serves the connections `listener` accepts, at most as many at once as `connections` has
/// permits, one past them closed unanswered, and each closed where it does not complete its TLS
/// handshake, where `tls` is given, or send a request's headers within `header_read`. Over TLS a
/// connection that does not speak it, plain HTTP included, is closed unanswered.
async fn accept(
    listener: TcpListener,
    app: Router,
    tls: Option<tokio_rustls::TlsAcceptor>,
    header_read: Duration,
    connections: Arc<Semaphore>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                // Out of descriptors, for instance: try again shortly rather than spin.
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let slot = Slot(Arc::new(std::sync::Mutex::new(Some(permit))));
        let service = TowerToHyperService::new(app.clone().layer(axum::Extension(slot.clone())));
        let tls = tls.clone();
        tokio::spawn(async move {
            match tls {
                None => serve(TokioIo::new(stream), service, header_read).await,
                Some(acceptor) => {
                    let handshake = tokio::time::timeout(header_read, acceptor.accept(stream));
                    if let Ok(Ok(stream)) = handshake.await {
                        serve(TokioIo::new(stream), service, header_read).await;
                    }
                }
            }
            slot.release();
        });
    }
}

/// The permit a connection holds among those served at once, which a request that waits gives
/// up before it waits.
#[derive(Clone)]
struct Slot(Arc<std::sync::Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>);

impl Slot {
    fn release(&self) {
        self.0.lock().unwrap().take();
    }
}

/// Serves HTTP/1.1 on `io`, with its upgrade to a WebSocket, closing it where it does not send a
/// request's headers within `header_read`.
async fn serve<I>(io: I, service: TowerToHyperService<Router>, header_read: Duration)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_read);
    let _ = builder.serve_connection(io, service).with_upgrades().await;
}

/// A refusal: the status and why, in words for the developer.
fn refuse(status: StatusCode, why: &'static str) -> Response {
    (status, why).into_response()
}

/// Answers one request, after the checks of ADR-0036 and with the headers of ADR-0035 and
/// ADR-0036 on every answer.
async fn handle(State(server): State<Arc<Server>>, request: Request) -> Response {
    let host = header_str(request.headers(), header::HOST).map(str::to_string);
    // On the loopback addresses the names of the machine itself; over TLS a name the
    // certificate carries. Any other is what a page reaching the server through DNS rebinding
    // sends (ADR-0036).
    let named = host.as_deref().is_some_and(|host| match &server.tls {
        None => auth::loopback_host(host),
        Some(tls) => auth::host_name(host).is_some_and(|name| tls.names(name)),
    });
    let mut response = match host {
        Some(ref host) if named => admit(&server, host, request).await,
        _ if server.tls.is_some() => refuse(
            StatusCode::FORBIDDEN,
            "This server answers only under a name its certificate carries",
        ),
        _ => refuse(
            StatusCode::FORBIDDEN,
            "This server answers only under the name localhost",
        ),
    };
    let headers = response.headers_mut();
    let csp = web::csp(
        host.as_deref().filter(|_| named).unwrap_or("localhost"),
        server.tls.is_some(),
        &server.script_hashes,
    );
    if let Ok(csp) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    for (name, value) in [
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CACHE_CONTROL, "no-store"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The request to `host`, a name of the machine itself, checked for its origin and its session
/// before anything is served or read (ADR-0036).
async fn admit(server: &Server, host: &str, request: Request) -> Response {
    let own = auth::own_origin(host, server.tls.is_some());
    let origin = header_str(request.headers(), header::ORIGIN);
    if origin.is_some_and(|o| !o.eq_ignore_ascii_case(&own)) {
        return refuse(
            StatusCode::FORBIDDEN,
            "Requests from another origin are refused",
        );
    }
    let path = request.uri().path().to_string();
    if let Some(token) = auth::query_param(request.uri().query(), "token") {
        // The right token is answered at once; a wrong one waits for its turn, one at a time for
        // the whole server, and then for what the wrong ones before it earned (ADR-0036).
        return match server.gate.exchange(token) {
            auth::Exchange::Session(id) => Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, "/")
                .header(header::SET_COOKIE, server.gate.set_cookie(&id))
                .body(Body::empty())
                .unwrap(),
            auth::Exchange::Refused { failures } => {
                let wait = auth::wait_after(failures - 1);
                if !wait.is_zero() {
                    // A wrong token that waits holds no connection slot, so that waiting ones
                    // never keep the right token out; only a bounded number wait, and one past
                    // them is refused at once with when to try again.
                    if let Some(slot) = request.extensions().get::<Slot>() {
                        slot.release();
                    }
                    let Ok(_waiting) = Arc::clone(&server.waiting).try_acquire_owned() else {
                        let mut refused = refuse(
                            StatusCode::TOO_MANY_REQUESTS,
                            "Too many attempts with a wrong connection token",
                        );
                        let headers = refused.headers_mut();
                        headers.insert(
                            header::RETRY_AFTER,
                            HeaderValue::from(wait.as_secs().max(1)),
                        );
                        headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
                        return refused;
                    };
                    let _turn = server.token_turn.lock().await;
                    tokio::time::sleep(wait).await;
                }
                // Logged at each doubling, never what the attempt carried.
                if failures.is_power_of_two() {
                    eprintln!("ocelli-server: {failures} attempts with a wrong connection token");
                }
                let mut refused = refuse(
                    StatusCode::FORBIDDEN,
                    "The connection token is not this server's",
                );
                // The connection ends with the answer: one that gave up its slot is not served
                // again.
                refused
                    .headers_mut()
                    .insert(header::CONNECTION, HeaderValue::from_static("close"));
                refused
            }
        };
    }
    if !server
        .gate
        .admits(header_str(request.headers(), header::COOKIE))
    {
        return refuse(
            StatusCode::FORBIDDEN,
            "Open the URL ocelli-server printed at its start",
        );
    }
    if path == "/ws" {
        // A handshake must name its origin, and it must be the server's own: a WebSocket is not
        // bound by the same-origin policy, and the browser sends the cookie whatever page opens
        // it (ADR-0036).
        if origin.is_none() {
            return refuse(
                StatusCode::FORBIDDEN,
                "A WebSocket without an origin is refused",
            );
        }
        let (mut parts, _) = request.into_parts();
        let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(upgrade) => upgrade,
            Err(rejection) => return rejection.into_response(),
        };
        let Ok(tab) = Arc::clone(&server.tabs).try_acquire_owned() else {
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "Too many tabs are open");
        };
        let (root, limits) = (Arc::clone(&server.root), server.limits);
        let (sessions, writes) = (server.sessions.clone(), server.writes.clone());
        return upgrade
            .max_message_size(socket::MAX_MESSAGE)
            .on_upgrade(move |ws| async move {
                socket::serve(ws, root, limits, sessions, writes).await;
                drop(tab);
            });
    }
    let name = match path.trim_start_matches('/') {
        "" => "index.html",
        name => name,
    };
    match web::file(name) {
        Some(bytes) => (
            [(header::CONTENT_TYPE, web::content_type(name))],
            Body::from(bytes),
        )
            .into_response(),
        None if !web::built() => refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "This ocelli-server was built without the web interface",
        ),
        None => refuse(StatusCode::NOT_FOUND, "Not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    // Verifies: ADR-0037
    fn the_server_root_is_required() {
        assert_eq!(Options::parse(args(&["ocelli-server"])).unwrap_err(), USAGE);
        assert_eq!(
            Options::parse(args(&["ocelli-server", "--root", "/r", "--port", "8000"])),
            Ok(Options {
                root: "/r".into(),
                port: 8000,
                limits: Limits::default(),
                listen: None,
                allow_writes: false,
            })
        );
        assert_eq!(
            Options::parse(args(&["ocelli-server", "--root", "/r"])),
            Ok(Options {
                root: "/r".into(),
                port: 0,
                limits: Limits::default(),
                listen: None,
                allow_writes: false,
            })
        );
        assert!(Options::parse(args(&["ocelli-server", "--root", "/r", "--port", "x"])).is_err());
        assert!(Options::parse(args(&["ocelli-server", "--root", "/r", "extra"])).is_err());
    }

    #[test]
    // Verifies: ADR-0035
    fn writes_are_allowed_only_by_the_switch_given_at_the_start() {
        let allowed = |list: &[&str]| {
            let mut all = vec!["ocelli-server"];
            all.extend(list);
            Options::parse(args(&all)).map(|o| o.allow_writes)
        };
        assert_eq!(allowed(&["--root", "/r"]), Ok(false));
        assert_eq!(allowed(&["--root", "/r", "--allow-writes"]), Ok(true));
        assert_eq!(allowed(&["--allow-writes", "--root", "/r"]), Ok(true));
        assert!(allowed(&["--root", "/r", "--allow-writes=no"]).is_err());
        assert!(USAGE.contains("[--allow-writes]"), "{USAGE}");
    }

    #[test]
    // Verifies: ADR-0036
    fn a_named_address_takes_its_port_or_the_one_given_and_a_certificate_comes_with_its_key() {
        let listen = |list: &[&str]| {
            let mut all = vec!["ocelli-server", "--root", "/r"];
            all.extend(list);
            Options::parse(args(&all)).map(|o| o.listen)
        };
        let at = |address: &str, cert: Option<(&str, &str)>| {
            Ok(Some(Listen {
                address: address.parse().unwrap(),
                cert: cert.map(|(c, k)| (c.into(), k.into())),
                keep_in: None,
            }))
        };
        assert_eq!(
            listen(&["--listen", "192.0.2.1:8443"]),
            at("192.0.2.1:8443", None)
        );
        assert_eq!(
            listen(&["--port", "9", "--listen", "[::1]"]),
            at("[::1]:9", None)
        );
        assert_eq!(listen(&["--listen", "::"]), at("[::]:0", None));
        assert_eq!(
            listen(&["--listen", "0.0.0.0:1", "--key", "k", "--cert", "c"]),
            at("0.0.0.0:1", Some(("c", "k")))
        );
        for wrong in [
            &["--listen", "host.example:1"][..],
            &["--listen", "0.0.0.0:1", "--cert", "c"],
            &["--listen", "0.0.0.0:1", "--key", "k"],
            &["--cert", "c", "--key", "k"],
            &["--listen"],
        ] {
            assert!(listen(wrong).is_err(), "{wrong:?}");
        }
    }
}
