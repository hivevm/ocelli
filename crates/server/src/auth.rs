//! Who the remote server admits (ADR-0038): a browser that carries the session cookie it got for
//! the connection token, from the server's own origin, under a name of the machine itself. Every
//! function here decides on values; the server applies them to each request before it serves a
//! file or reads a command.

use std::sync::Mutex;

/// The name of the session cookie. Plain HTTP on `localhost` leaves it without `Secure`, and so
/// without the `__Host-` prefix, which requires it (ADR-0038).
pub const COOKIE: &str = "ocelli-session";

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
/// (ADR-0038).
pub fn same(a: &[u8; SECRET], b: &[u8; SECRET]) -> bool {
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// The connection token printed at start, and the session ids it has been exchanged for, kept in
/// memory so that a restart ends every session (ADR-0038).
pub struct Gate {
    token: [u8; SECRET],
    sessions: Mutex<Vec<[u8; SECRET]>>,
}

impl Gate {
    pub fn new() -> Self {
        Gate {
            token: secret(),
            sessions: Mutex::new(Vec::new()),
        }
    }

    /// The token as the printed URL carries it.
    pub fn token(&self) -> String {
        hex(&self.token)
    }

    /// A new session id for `token`, or `None` where it is not this server's.
    pub fn exchange(&self, token: &str) -> Option<String> {
        let token = unhex(token)?;
        if !same(&token, &self.token) {
            return None;
        }
        let id = secret();
        self.sessions.lock().unwrap().push(id);
        Some(hex(&id))
    }

    /// Whether the `Cookie` header carries a session id this server gave out. Every id kept is
    /// compared, each in constant time.
    pub fn admits(&self, cookie_header: Option<&str>) -> bool {
        let Some(header) = cookie_header else {
            return false;
        };
        let sessions = self.sessions.lock().unwrap();
        cookies(header, COOKIE)
            .filter_map(unhex)
            .any(|id| sessions.iter().fold(false, |found, s| found | same(&id, s)))
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
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

/// The `Set-Cookie` header that hands a browser its session id: `HttpOnly`, `SameSite=Strict`,
/// `Path=/`, no `Domain`, and no expiry (ADR-0038).
pub fn set_cookie(id: &str) -> String {
    format!("{COOKIE}={id}; HttpOnly; SameSite=Strict; Path=/")
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
/// (ADR-0038).
pub fn loopback_host(host: &str) -> bool {
    // The name, an IPv6 address with its brackets, and what follows it: nothing or `:port`.
    let end = if host.starts_with('[') {
        match host.find(']') {
            Some(i) => i + 1,
            None => return false,
        }
    } else {
        host.find(':').unwrap_or(host.len())
    };
    let (name, port) = host.split_at(end);
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    port_ok
        && ["localhost", "127.0.0.1", "[::1]"]
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
}

/// The origin a request to `host` over plain HTTP comes from: the scheme of the listener with the
/// `Host` it names (ADR-0038).
pub fn own_origin(host: &str) -> String {
    format!("http://{}", host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verifies: ADR-0038
    fn a_token_opens_a_session_and_only_its_cookie_is_admitted() {
        let gate = Gate::new();
        assert_eq!(gate.token().len(), 64, "256 bits");
        assert!(!gate.admits(None));
        let id = gate.exchange(&gate.token()).expect("the server's token");
        assert!(gate.admits(Some(&format!("theme=dark; {COOKIE}={id}"))));
        assert!(!gate.admits(Some(&format!("{COOKIE}={}", gate.token()))));
        assert!(!gate.admits(Some(&format!("other={id}"))));
        let forged = hex(&secret());
        assert!(!gate.admits(Some(&format!("{COOKIE}={forged}"))));
    }

    #[test]
    // Verifies: ADR-0038
    fn a_wrong_or_malformed_token_opens_no_session() {
        let gate = Gate::new();
        assert_eq!(gate.exchange(&hex(&secret())), None);
        assert_eq!(gate.exchange(""), None);
        assert_eq!(gate.exchange(&gate.token().to_ascii_uppercase()), None);
        assert_eq!(gate.exchange(&format!("{}0", gate.token())), None);
        assert_eq!(gate.exchange(&"zz".repeat(32)), None);
    }

    #[test]
    // Verifies: ADR-0038
    fn two_starts_draw_two_tokens() {
        assert_ne!(Gate::new().token(), Gate::new().token());
    }

    #[test]
    // Verifies: ADR-0038
    fn the_cookie_is_http_only_strict_and_for_the_whole_server() {
        let cookie = set_cookie("ab");
        assert_eq!(
            cookie,
            "ocelli-session=ab; HttpOnly; SameSite=Strict; Path=/"
        );
        assert!(!cookie.contains("Domain") && !cookie.contains("Expires"));
    }

    #[test]
    // Verifies: ADR-0038
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
