//! The web interface's build, embedded in the binary, and the policy it is served under: the
//! headers ADR-0023 sets for the WebView, as HTTP headers (ADR-0037), with the framing and
//! referrer rules of ADR-0038.

use base64::Engine as _;
use sha2::{Digest, Sha256};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/web.rs"));
}

/// Whether the binary carries a build of the web interface.
pub fn built() -> bool {
    file("index.html").is_some()
}

/// The embedded file at `path`, relative to the build's root.
pub fn file(path: &str) -> Option<&'static [u8]> {
    embedded::FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}

/// The media type of a file of the build, by its extension.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// The hashes of the inline scripts of `html`, the theme bootstrap of `index.html` among them,
/// as a Content-Security-Policy source each: the bundle's own scripts, which `'self'` does not
/// cover.
pub fn inline_script_hashes(html: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let Some(open_end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + open_end];
        let body_start = start + open_end + 1;
        let Some(len) = rest[body_start..].find("</script>") else {
            break;
        };
        if !tag.contains(" src=") {
            let digest = Sha256::digest(&rest.as_bytes()[body_start..body_start + len]);
            hashes.push(format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(digest)
            ));
        }
        rest = &rest[body_start + len..];
    }
    hashes
}

/// The Content-Security-Policy of every answer to a request made to `host`: the bundle's own
/// scripts and styles, a connection to the server itself, avatars from Gravatar and nothing else
/// beyond it (ADR-0023, ADR-0037), and no page that frames the server (ADR-0038). Over TLS the
/// socket is `wss`.
pub fn csp(host: &str, tls: bool, script_hashes: &[String]) -> String {
    let scripts = std::iter::once("'self'".to_string())
        .chain(script_hashes.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let ws = if tls { "wss" } else { "ws" };
    format!(
        "default-src 'self'; script-src {scripts}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' https://www.gravatar.com; connect-src 'self' {ws}://{host}; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verifies: ADR-0037, ADR-0038, ADR-0023
    fn the_policy_grants_one_origin_beyond_the_server_and_no_framing() {
        assert!(csp("h:1", true, &[]).contains("; connect-src 'self' wss://h:1;"));
        let policy = csp("localhost:8000", false, &["'sha256-x'".into()]);
        let directives: Vec<_> = policy.split(';').map(str::trim).collect();
        assert!(directives.contains(&"script-src 'self' 'sha256-x'"));
        assert!(directives.contains(&"frame-ancestors 'none'"));
        assert!(directives.contains(&"connect-src 'self' ws://localhost:8000"));
        let external: Vec<_> = directives
            .iter()
            .flat_map(|d| {
                let mut words = d.split_whitespace();
                let name = words.next().unwrap_or_default();
                words
                    .filter(|w| w.starts_with("http") || w.starts_with("ws"))
                    .map(move |w| format!("{name} {w}"))
            })
            .collect();
        assert_eq!(
            external,
            [
                "img-src https://www.gravatar.com",
                "connect-src ws://localhost:8000"
            ]
        );
    }

    #[test]
    // Verifies: ADR-0037
    fn each_inline_script_is_hashed_and_a_bundled_one_is_not() {
        let html = "<script>\n  a();\n</script>\n<script type=\"module\" src=\"/x.js\"></script>";
        let hashes = inline_script_hashes(html);
        let digest = Sha256::digest(b"\n  a();\n");
        let expected = base64::engine::general_purpose::STANDARD.encode(digest);
        assert_eq!(hashes, [format!("'sha256-{expected}'")]);
    }

    #[test]
    fn the_build_s_files_are_served_with_their_media_types() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/x.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("assets/x"), "application/octet-stream");
    }
}
