//! Who the remote server admits (ADR-0036): a browser that carries the session cookie it got for
//! the connection token, from the server's own origin, under a name of the machine itself. Every
//! function here decides on values; the server applies them to each request before it serves a
//! file or reads a command.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

/// The name of the session cookie. Plain HTTP on `localhost` leaves it without `Secure`, and so
/// without the `__Host-` prefix, which requires it (ADR-0036).
pub const COOKIE: &str = "ocelli-session";

/// The name of the session cookie over TLS, where it is `Secure` and so carries the `__Host-`
/// prefix, which binds it to the origin that set it (ADR-0036).
pub const SECURE_COOKIE: &str = "__Host-ocelli-session";

/// The name of the session cookie over TLS or over plain HTTP.
pub fn cookie_name(tls: bool) -> &'static str {
    if tls { SECURE_COOKIE } else { COOKIE }
}

/// The bytes of a connection token or a session id: 256 bits.
const SECRET: usize = 32;

/// A random secret of 256 bits from the operating system's random source.
pub fn secret() -> [u8; SECRET] {
    let mut bytes = [0; SECRET];
    getrandom::fill(&mut bytes).expect("the operating system's random source answers");
    bytes
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes a secret's lowercase hexadecimal spells, `None` for anything else.
fn unhex(text: &str) -> Option<[u8; SECRET]> {
    let digits = text.as_bytes();
    if digits.len() != 2 * SECRET {
        return None;
    }
    let mut bytes = [0; SECRET];
    for (byte, pair) in bytes.iter_mut().zip(digits.chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        if pair.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

/// Whether two secrets are equal, in a time that does not depend on where they differ
/// (ADR-0036).
pub fn same(a: &[u8; SECRET], b: &[u8; SECRET]) -> bool {
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// The connection token printed at start, and the session ids it has been exchanged for, kept in
/// memory so that a restart ends every session (ADR-0036).
pub struct Gate {
    token: [u8; SECRET],
    /// The session ids given out, oldest first, at most `max_sessions` of them.
    sessions: Mutex<VecDeque<[u8; SECRET]>>,
    max_sessions: usize,
    /// The wrong tokens since the last right one, counted for the whole server (ADR-0036).
    failures: Mutex<u32>,
    /// The name of the session cookie it admits: `SECURE_COOKIE` over TLS.
    cookie: &'static str,
}

/// What a token is exchanged for.
#[derive(Debug, PartialEq, Eq)]
pub enum Exchange {
    /// A new session id.
    Session(String),
    /// None: the token is not this server's, the `failures`-th wrong one in a row.
    Refused { failures: u32 },
}

/// The wrong tokens answered at once; past them each answer waits.
pub const FREE_FAILURES: u32 = 5;
/// The wait after the first wrong token past the free ones, doubling with each next one.
pub const FIRST_WAIT: Duration = Duration::from_millis(250);
/// The longest wait.
pub const MAX_WAIT: Duration = Duration::from_secs(8);

/// How long the server waits before it answers a token after `failures` wrong ones in a row:
/// nothing for the first few, then a wait doubling up to a cap (ADR-0036). With 256 bits a token
/// is not guessed either way; the wait guards the log and the processor.
pub fn wait_after(failures: u32) -> Duration {
    if failures < FREE_FAILURES {
        return Duration::ZERO;
    }
    let doublings = (failures - FREE_FAILURES).min(16);
    (FIRST_WAIT * 2u32.pow(doublings)).min(MAX_WAIT)
}

impl Gate {
    /// A gate with a new token that keeps at most `max_sessions` session ids, the oldest giving
    /// way to a new one.
    pub fn new(max_sessions: usize) -> Self {
        Gate {
            token: secret(),
            sessions: Mutex::new(VecDeque::new()),
            max_sessions: max_sessions.max(1),
            failures: Mutex::new(0),
            cookie: COOKIE,
        }
    }

    /// The gate of a server that speaks TLS, whose session cookie is `SECURE_COOKIE`.
    pub fn over_tls(self) -> Self {
        Gate {
            cookie: SECURE_COOKIE,
            ..self
        }
    }

    /// The token as the printed URL carries it.
    pub fn token(&self) -> String {
        hex(&self.token)
    }

    /// How long the next token's answer waits.
    pub fn wait(&self) -> Duration {
        wait_after(*self.failures.lock().unwrap())
    }

    /// A new session id for `token`, or the count of wrong tokens where it is not this server's.
    pub fn exchange(&self, token: &str) -> Exchange {
        let right = unhex(token).is_some_and(|t| same(&t, &self.token));
        let mut failures = self.failures.lock().unwrap();
        if !right {
            *failures = failures.saturating_add(1);
            return Exchange::Refused {
                failures: *failures,
            };
        }
        *failures = 0;
        let id = secret();
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.len() >= self.max_sessions {
            sessions.pop_front();
        }
        sessions.push_back(id);
        Exchange::Session(hex(&id))
    }

    /// Whether the `Cookie` header carries a session id this server gave out. Every id kept is
    /// compared, each in constant time.
    pub fn admits(&self, cookie_header: Option<&str>) -> bool {
        let Some(header) = cookie_header else {
            return false;
        };
        let sessions = self.sessions.lock().unwrap();
        cookies(header, self.cookie)
            .filter_map(unhex)
            .any(|id| sessions.iter().fold(false, |found, s| found | same(&id, s)))
    }

    /// The `Set-Cookie` header that hands a browser the session id `id`: `HttpOnly`,
    /// `SameSite=Strict`, `Path=/`, no `Domain`, no expiry, and over TLS `Secure` with the
    /// `__Host-` prefix (ADR-0036).
    pub fn set_cookie(&self, id: &str) -> String {
        let secure = if self.cookie == SECURE_COOKIE {
            "; Secure"
        } else {
            ""
        };
        format!(
            "{}={id}; HttpOnly; SameSite=Strict; Path=/{secure}",
            self.cookie
        )
    }
}

/// The values of the cookies named `name` in a `Cookie` header.
fn cookies<'a>(header: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(move |(n, _)| *n == name)
        .map(|(_, v)| v)
}

/// The value of the query parameter `name`, as it stands in the URL.
pub fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
}

/// Whether `host`, a `Host` header, names the machine itself, at any port so that a tunnel's
/// port works: anything else is what a page reaching the server through DNS rebinding sends
/// (ADR-0036).
pub fn loopback_host(host: &str) -> bool {
    host_name(host).is_some_and(|name| {
        ["localhost", "127.0.0.1", "[::1]"]
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
    })
}

/// The name `host`, a `Host` header, names, an IPv6 address with its brackets, where what
/// follows it is nothing or `:port`; `None` for anything else.
pub fn host_name(host: &str) -> Option<&str> {
    let end = if host.starts_with('[') {
        host.find(']')? + 1
    } else {
        host.find(':').unwrap_or(host.len())
    };
    let (name, port) = host.split_at(end);
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    (port_ok && !name.is_empty()).then_some(name)
}

/// The origin a request to `host` comes from: the scheme of the listener, `https` over TLS, with
/// the `Host` it names (ADR-0036).
pub fn own_origin(host: &str, tls: bool) -> String {
    let scheme = if tls { "https" } else { "http" };
    format!("{scheme}://{}", host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verifies: ADR-0036
    fn a_token_opens_a_session_and_only_its_cookie_is_admitted() {
        let gate = Gate::new(8);
        assert_eq!(gate.token().len(), 64, "256 bits");
        assert!(!gate.admits(None));
        let Exchange::Session(id) = gate.exchange(&gate.token()) else {
            panic!("the server's token opens a session")
        };
        assert!(gate.admits(Some(&format!("theme=dark; {COOKIE}={id}"))));
        assert!(!gate.admits(Some(&format!("{COOKIE}={}", gate.token()))));
        assert!(!gate.admits(Some(&format!("other={id}"))));
        let forged = hex(&secret());
        assert!(!gate.admits(Some(&format!("{COOKIE}={forged}"))));
    }

    #[test]
    // Verifies: ADR-0036
    fn a_wrong_or_malformed_token_opens_no_session_and_is_counted() {
        let gate = Gate::new(8);
        let wrong = [
            hex(&secret()),
            String::new(),
            gate.token().to_ascii_uppercase(),
            format!("{}0", gate.token()),
            "zz".repeat(32),
        ];
        for (n, token) in wrong.iter().enumerate() {
            assert_eq!(
                gate.exchange(token),
                Exchange::Refused {
                    failures: n as u32 + 1
                }
            );
        }
        assert_eq!(gate.wait(), FIRST_WAIT);
        // The right token still opens a session, and the count starts again.
        assert!(matches!(gate.exchange(&gate.token()), Exchange::Session(_)));
        assert_eq!(gate.wait(), Duration::ZERO);
    }

    #[test]
    // Verifies: ADR-0036
    fn the_wait_starts_past_the_free_failures_and_doubles_up_to_the_cap() {
        assert_eq!(wait_after(0), Duration::ZERO);
        assert_eq!(wait_after(FREE_FAILURES - 1), Duration::ZERO);
        assert_eq!(wait_after(FREE_FAILURES), FIRST_WAIT);
        assert_eq!(wait_after(FREE_FAILURES + 1), FIRST_WAIT * 2);
        assert_eq!(wait_after(FREE_FAILURES + 4), FIRST_WAIT * 16);
        assert_eq!(wait_after(FREE_FAILURES + 5), MAX_WAIT);
        assert_eq!(wait_after(u32::MAX), MAX_WAIT);
    }

    #[test]
    // Verifies: ADR-0036
    fn the_oldest_session_gives_way_past_the_bound() {
        let gate = Gate::new(2);
        let ids: Vec<String> = (0..3)
            .map(|_| match gate.exchange(&gate.token()) {
                Exchange::Session(id) => id,
                refused => panic!("{refused:?}"),
            })
            .collect();
        let admits = |id: &String| gate.admits(Some(&format!("{COOKIE}={id}")));
        assert!(!admits(&ids[0]));
        assert!(admits(&ids[1]) && admits(&ids[2]));
    }

    #[test]
    // Verifies: ADR-0036
    fn two_starts_draw_two_tokens() {
        assert_ne!(Gate::new(1).token(), Gate::new(1).token());
    }

    #[test]
    // Verifies: ADR-0036
    fn the_cookie_is_http_only_strict_and_for_the_whole_server() {
        let cookie = Gate::new(1).set_cookie("ab");
        assert_eq!(
            cookie,
            "ocelli-session=ab; HttpOnly; SameSite=Strict; Path=/"
        );
        assert!(!cookie.contains("Domain") && !cookie.contains("Expires"));
    }

    #[test]
    // Verifies: ADR-0036
    fn over_tls_the_cookie_is_secure_and_bound_to_its_origin_by_its_prefix() {
        let gate = Gate::new(8).over_tls();
        assert_eq!(
            gate.set_cookie("ab"),
            "__Host-ocelli-session=ab; HttpOnly; SameSite=Strict; Path=/; Secure"
        );
        let Exchange::Session(id) = gate.exchange(&gate.token()) else {
            panic!("the server's token opens a session")
        };
        assert!(gate.admits(Some(&format!("{SECURE_COOKIE}={id}"))));
        // The name without its prefix, which a page of another origin on the machine's name
        // could have set, is not this server's cookie.
        assert!(!gate.admits(Some(&format!("{COOKIE}={id}"))));
        assert_eq!(own_origin("Host:1", true), "https://host:1");
        assert_eq!(own_origin("host:1", false), "http://host:1");
    }

    #[test]
    // Verifies: ADR-0036
    fn only_names_of_the_machine_itself_are_hosts_at_any_port() {
        for host in [
            "localhost",
            "localhost:8000",
            "127.0.0.1:1",
            "[::1]:65535",
            "[::1]",
            "LOCALHOST:9",
        ] {
            assert!(loopback_host(host), "{host}");
        }
        for host in [
            "evil.example",
            "evil.example:8000",
            "localhost.evil.example",
            "127.0.0.2:80",
            "[::2]:80",
            "localhost:",
            "localhost:80x",
            "[::1",
            "",
        ] {
            assert!(!loopback_host(host), "{host}");
        }
    }

    #[test]
    fn the_query_names_its_parameters() {
        assert_eq!(query_param(Some("a=1&token=x"), "token"), Some("x"));
        assert_eq!(query_param(Some("tokens=x"), "token"), None);
        assert_eq!(query_param(None, "token"), None);
    }
}
