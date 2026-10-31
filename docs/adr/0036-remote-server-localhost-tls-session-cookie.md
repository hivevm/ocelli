# ADR-0036: The remote server listens on localhost by default and on a network only over TLS, and admits a browser by a token per start exchanged for a session cookie, from its own origin only

- **Status:** 🟢 accepted
- **Date:** 2026-10-08
- **Deciders:** Markus Brigl
- **Applies to:** the listening address, TLS, certificate, authentication, session cookie, Origin and Host checks, and rate limit of `ocelli-server`, and the tests of Q-5

## Context

Q-5 says the remote server hands control of a machine to whoever reaches it, and measures it so:
every request without valid authentication is refused, and the server accepts none from a web
page of another origin. The Threats name the boundary between the browser and the remote server:
anyone who can reach the server over the network, any web page open in the developer's browser,
and other users of the machine the server runs on are untrusted. The server serves one developer,
not a team (Non-Goals), and the Constraints and Assumptions model it on VS Code's remote
development: a long-running process the developer starts under their own account.

The server carries the desktop host's commands over one WebSocket per browser tab
([ADR-0035](0035-remote-server-websocket-per-tab.md)). Browsers send cookies with a WebSocket
handshake whatever page opens it, and a WebSocket is not bound by the same-origin policy: a page
of another origin that can open a socket to the server, carrying the developer's cookie, controls
it. A server on `localhost` is reachable from every page in the developer's browser and from
every account on the machine, and a page can reach it under a name of its own through DNS
rebinding.

A developer reaches a server on another machine in one of two ways: an SSH tunnel to a server
listening on that machine's `localhost`, as VS Code's remote development does, or the network
directly. A certificate from a public authority is often not to be had for a machine on a
private network.

## Decision

We will have `ocelli-server` listen on `localhost` without TLS unless the developer names an
address with `--listen`, and then always over TLS, and admit a browser only by a random token
printed at each start, which its first request exchanges for a session cookie, refusing every
request and WebSocket handshake without that cookie or from another origin.

- **Where it listens.** By default on the loopback addresses only, over plain HTTP. `--listen
  <address>` binds that address and always serves TLS, the loopback address included; there is
  no way to listen on a network without TLS.
- **TLS.** Through `rustls`, with its defaults: TLS 1.2 and 1.3 and no obsolete ciphers. With
  `--cert` and `--key` the server uses the certificate and key the developer gives. Without them
  it generates a self-signed certificate on its first start with `--listen`, for the host name and
  the address it listens on, and keeps it with its key in the user's configuration directory,
  readable by the developer's account alone, reusing it on every later start. At each start it
  prints the certificate's SHA-256 fingerprint, for the developer to compare with what the browser
  shows before accepting it.
- **The connection token.** At each start the server draws 256 bits from the operating system's
  random source and prints, once, the URL that carries it. The token is valid until the server
  stops; it never appears in the command line, a log, or a file.
- **The session cookie.** A request that carries the valid token gets a new random session id in
  a cookie that is `HttpOnly`, `SameSite=Strict`, `Path=/`, has no `Domain` and no expiry, and,
  over TLS, is `Secure` with the `__Host-` prefix; the answer redirects to the same address
  without the token. The server keeps its session ids in memory, so a restart ends every session.
  Tokens and session ids are compared in constant time.
- **What is refused.** Every HTTP request and every WebSocket handshake that carries no valid
  session cookie is refused, the token exchange aside, before any file is served or any command
  read: the files of the web interface included.
- **Origin and Host.** A WebSocket handshake whose `Origin` is not the origin the request was made
  to, the scheme of the listener with the `Host` it names, is refused, and so is one without an
  `Origin`; an HTTP request that carries an `Origin` of another origin is refused. On `localhost`,
  a request whose `Host` is not `localhost`, `127.0.0.1`, or `[::1]`, at any port so that a
  tunnel's port works, is refused, which a page reaching the server through DNS rebinding sends.
- **Headers.** Every answer carries `Referrer-Policy: no-referrer`, so the URL with the token
  leaves no trace in a request to another site, and a Content-Security-Policy with
  `frame-ancestors 'none'`, so no page frames the server.
- **Failed attempts.** A request with a token that is not the server's is refused and counted; past
  a few failures the server waits longer before it answers each next one, the wait doubling up to
  a cap, counted for the whole server rather than per address, and it logs that attempts fail
  without logging what they carried.

**Out of scope:** which repositories an authenticated browser may open, which is ADR-0037;
accounts, users, or roles, which a server for one developer does not have; logging out, which a
restart does; a certificate from a public authority, which the developer gives with `--cert` and
`--key` like any other.

## Alternatives considered

- **Only `localhost`, the network through SSH** — the smallest surface: no TLS, no certificate,
  the tunnel authenticating the developer; but the developer whose machine is reached without SSH,
  in a container or on a private network, has no way in, and the token is needed anyway, since
  every account and every page on the machine reaches `localhost`.
- **Passwords** — familiar and stable across restarts, but a password is chosen, reused, stored
  somewhere to be checked, and guessed; a random token per start is stronger than any password a
  developer picks and needs no storage.
- **No authentication of its own, behind a proxy or the tunnel** — what `code serve-web
  --without-connection-token` allows; it puts Q-5 outside Ocelli, and on `localhost` it is not
  met at all, since other accounts and other pages reach it.
- **The token on every request, no cookie** — no session state, but the token then lives in the
  web interface's script, in every URL or WebSocket message, and in the browser's history; a
  cookie the script cannot read keeps it out of reach of what the script draws, a repository's
  content included.
- **`SameSite=Lax`, as VS Code's server sets its token cookie** — survives a link from another
  site, but such a link is exactly the cross-site request Q-5 refuses.
- **A token valid across restarts, kept in a file** — the URL stays in the browser's bookmarks,
  but a file whose content controls the machine, which a backup or another account may read.
- **Plain HTTP on a network address, TLS the developer's business** — simpler, but the token and
  the cookie cross the network readable to anyone on it.
- **OpenSSL for TLS** — the most deployed library, but a C library and a system dependency on
  three platforms, where `rustls` is Rust with no unsafe features or obsolete cryptography by
  default.
- **A rate limit per address** — keeps one attacker from slowing the developer, but an attacker
  with many addresses then guesses at full speed; with a token of 256 bits guessing is hopeless
  either way, and the limit guards the log and the processor, so one counter is enough.

## Sources / Prior art

- VS Code, `code serve-web`: `--host` "defaults to 'localhost'", `--connection-token` "a secret
  that must be included with all requests", a random UUID when none is given, printed in the URL as
  `?tkn=`, and `--without-connection-token` "only use this if the connection is secured by other
  means" (`cli/src/commands/args.rs` and `cli/src/commands/serve_web.rs`, read 2026-10-08) —
  <https://github.com/microsoft/vscode/tree/main/cli/src/commands>
- VS Code server, `webClientServer.ts`: a token in the query is answered with a `vscode-tkn`
  cookie, `SameSite=Lax`, one week, and a redirect to the URL without the token (read 2026-10-08)
  — <https://github.com/microsoft/vscode/blob/main/src/vs/server/node/webClientServer.ts>
- Jupyter Server, *Security in the Jupyter Server*: a token generated at start, logged as a URL,
  and once used, a cookie in its place —
  <https://jupyter-server.readthedocs.io/en/latest/operators/security.html>;
  its `ip` defaults to `localhost`, and `check_origin` compares `Origin` with `Host`
  (`jupyter_server/serverapp.py`, `jupyter_server/base/handlers.py`, read 2026-10-08) —
  <https://github.com/jupyter-server/jupyter_server>
- OWASP, *WebSocket Security Cheat Sheet*: validate `Origin` on every handshake against an
  allowlist, Cross-Site WebSocket Hijacking through cookies the browser sends, `SameSite` cookies,
  no `ws://` in production —
  <https://cheatsheetseries.owasp.org/cheatsheets/WebSocket_Security_Cheat_Sheet.html>
- OWASP, *Cross-Site Request Forgery Prevention Cheat Sheet*: verifying the origin with standard
  headers; `SameSite=Strict`, and `SameSite` as defence in depth, not the defence —
  <https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html>
- OWASP, *Authentication Cheat Sheet*: throttling failed attempts, and counting them against the
  account rather than the address —
  <https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html>
- RFC 6455, section 10.2: a browser sends `Origin` with the handshake, and a server that objects
  to it answers with an error such as 403 and aborts —
  <https://www.rfc-editor.org/rfc/rfc6455#section-10.2>
- MDN, *Set-Cookie*: the `__Host-` prefix requires `Secure`, no `Domain`, and `Path=/`; the
  `Secure` requirement is relaxed on `localhost` —
  <https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie>
- rustls: TLS 1.2 and 1.3, "no unsafe features or obsolete cryptography by default" —
  <https://docs.rs/rustls/>; rcgen, which generates self-signed X.509 certificates —
  <https://docs.rs/rcgen/>
- Recalled, not verified for this record: DNS rebinding as the way a page reaches a service on
  `localhost` under its own name, and that browsers show a certificate's SHA-256 fingerprint in
  their certificate viewer.

## Consequences

- Positive: Q-5 holds on both listeners: no request reaches a file or a command without the
  cookie, the cookie never leaves for another site, and a page of another origin cannot open the
  socket even with it.
- Positive: the default start opens nothing to the network, and an SSH tunnel works without any
  setting, a tunnel's local port included.
- Positive: a developer on a network without SSH gets TLS with no certificate to make, and checks
  the self-signed one by its fingerprint once.
- Negative / trade-offs: the browser warns about the self-signed certificate on first use, and a
  developer who accepts it without comparing the fingerprint gains no protection from it.
- Negative / trade-offs: every restart prints a new URL, and the open tabs must be opened again
  with it.
- Negative / trade-offs: other accounts on the machine reach the `localhost` listener; the token
  keeps them out only as long as the terminal it is printed to stays the developer's.
- Negative / trade-offs: plain HTTP on `localhost` leaves the cookie without `Secure`, although
  browsers would accept it there.
- Follow-ups: how the server prints the URL and fingerprint when it is started by a script or a
  service rather than in a terminal; renewing a self-signed certificate before it expires.

## Enforcement

Tests in `ocelli-server` that cite this ADR and Q-5 with a `Verifies:` marker, against a server
started in the test:

- with no cookie, a file of the web interface, a WebSocket handshake, and a request with a
  forged session id are each refused, and nothing of the web interface is served;
- the URL with the token answers with a redirect and a cookie that is `HttpOnly` and
  `SameSite=Strict`, and over TLS `Secure` with the `__Host-` prefix; a wrong token sets none;
- a handshake with the cookie and an `Origin` of another origin, or none, is refused, and one from
  the server's own origin is accepted; an HTTP request with a foreign `Origin` is refused; on
  `localhost` a foreign `Host` is refused and `localhost` at another port is accepted;
- by default the server binds loopback addresses only; with `--listen` it speaks TLS and refuses
  plain HTTP; a self-signed certificate is written on first start with the developer's
  permissions only, reused on the second, and its printed fingerprint matches;
- after a few wrong tokens the next answer waits longer, and the valid token still opens a
  session afterwards; a new start refuses the previous start's cookie.
