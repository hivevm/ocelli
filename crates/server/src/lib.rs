//! The remote server (ADR-0037): it serves the web interface to a browser and carries the desktop
//! host's commands and events over one WebSocket per tab, answering them through the commands
//! both hosts share. It listens on the machine's loopback addresses and admits a browser only by
//! the connection token it prints, exchanged for a session cookie, from its own origin (ADR-0038),
//! and opens only the repositories below its server root (ADR-0039).

pub mod auth;
pub mod root;
pub mod socket;
pub mod web;

use auth::Gate;
use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequestParts, Request, State, ws::WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use root::Root;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

/// What the server is started with.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    /// The server root: required, there is no default (ADR-0039).
    pub root: PathBuf,
    /// The port on the loopback addresses; 0 lets the system choose a free one.
    pub port: u16,
}

pub const USAGE: &str = "Usage: ocelli-server --root <folder> [--port <port>]";

impl Options {
    /// The options of a command line, the program's name first.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
        let mut args = args.into_iter().skip(1);
        let (mut root, mut port) = (None, 0);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" => root = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
                "--port" => {
                    port = args
                        .next()
                        .and_then(|p| p.parse().ok())
                        .ok_or("--port takes a number from 0 to 65535")?
                }
                _ => return Err(format!("Unknown argument {arg}\n{USAGE}")),
            }
        }
        Ok(Options {
            root: root.ok_or(USAGE)?,
            port,
        })
    }
}

/// What every request is answered from.
struct Server {
    root: Arc<Root>,
    gate: Gate,
    /// The hashes of the inline scripts of the web interface's `index.html`.
    script_hashes: Vec<String>,
}

/// A server that runs: the port it listens on and the URL that carries its connection token.
pub struct Running {
    pub port: u16,
    pub url: String,
    pub root: PathBuf,
}

/// Starts the server on the loopback addresses (ADR-0038): IPv4 always, IPv6 where the machine
/// has it, on the same port.
pub async fn start(options: Options) -> Result<Running, String> {
    let root = Root::new(&options.root)?;
    let script_hashes = web::file("index.html")
        .map(|html| web::inline_script_hashes(&String::from_utf8_lossy(html)))
        .unwrap_or_default();
    let server = Arc::new(Server {
        root: Arc::new(root),
        gate: Gate::new(),
        script_hashes,
    });
    let v4 = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, options.port)))
        .await
        .map_err(|e| format!("Cannot listen on 127.0.0.1:{}: {e}", options.port))?;
    let port = v4.local_addr().map_err(|e| e.to_string())?.port();
    let v6 = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port)))
        .await
        .ok();
    let app = Router::new()
        .fallback(handle)
        .with_state(Arc::clone(&server));
    for listener in std::iter::once(v4).chain(v6) {
        let app = app.clone();
        tokio::spawn(async move { axum::serve(listener, app).await });
    }
    Ok(Running {
        port,
        url: format!("http://localhost:{port}/?token={}", server.gate.token()),
        root: server.root.path().to_path_buf(),
    })
}

/// A refusal: the status and why, in words for the developer.
fn refuse(status: StatusCode, why: &'static str) -> Response {
    (status, why).into_response()
}

/// Answers one request, after the checks of ADR-0038 and with the headers of ADR-0037 and
/// ADR-0038 on every answer.
async fn handle(State(server): State<Arc<Server>>, request: Request) -> Response {
    let host = header_str(request.headers(), header::HOST).map(str::to_string);
    let mut response = match host {
        Some(ref host) if auth::loopback_host(host) => admit(&server, host, request).await,
        _ => refuse(
            StatusCode::FORBIDDEN,
            "This server answers only under the name localhost",
        ),
    };
    let headers = response.headers_mut();
    let csp = web::csp(
        host.as_deref().unwrap_or("localhost"),
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
/// before anything is served or read (ADR-0038).
async fn admit(server: &Server, host: &str, request: Request) -> Response {
    let own = auth::own_origin(host);
    let origin = header_str(request.headers(), header::ORIGIN);
    if origin.is_some_and(|o| !o.eq_ignore_ascii_case(&own)) {
        return refuse(
            StatusCode::FORBIDDEN,
            "Requests from another origin are refused",
        );
    }
    let path = request.uri().path().to_string();
    if let Some(token) = auth::query_param(request.uri().query(), "token") {
        return match server.gate.exchange(token) {
            Some(id) => Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, "/")
                .header(header::SET_COOKIE, auth::set_cookie(&id))
                .body(Body::empty())
                .unwrap(),
            None => refuse(
                StatusCode::FORBIDDEN,
                "The connection token is not this server's",
            ),
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
        // it (ADR-0038).
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
        let root = Arc::clone(&server.root);
        return upgrade
            .max_message_size(socket::MAX_MESSAGE)
            .on_upgrade(move |ws| socket::serve(ws, root));
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
    // Verifies: ADR-0039
    fn the_server_root_is_required() {
        assert_eq!(Options::parse(args(&["ocelli-server"])).unwrap_err(), USAGE);
        assert_eq!(
            Options::parse(args(&["ocelli-server", "--root", "/r", "--port", "8000"])),
            Ok(Options {
                root: "/r".into(),
                port: 8000
            })
        );
        assert_eq!(
            Options::parse(args(&["ocelli-server", "--root", "/r"])),
            Ok(Options {
                root: "/r".into(),
                port: 0
            })
        );
        assert!(Options::parse(args(&["ocelli-server", "--root", "/r", "--port", "x"])).is_err());
        assert!(Options::parse(args(&["ocelli-server", "--root", "/r", "extra"])).is_err());
    }
}
