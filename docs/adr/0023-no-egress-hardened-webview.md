# ADR-0023: The front ends make no network request by default, and the WebView is granted only what it uses

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `src-tauri/tauri.conf.json`, the avatar toggle, and every outbound request the web interface and the terminal application can make

## Context

Ocelli reads a repository from disk through `git2` ([ADR-0019](0019-git2-repository-reader.md)),
and reaches the network only where the developer asks it to: fetching, pulling, and pushing to a
remote repository through the developer's own Git setup (G-12, Constraints). Nothing else Ocelli
shows needs the network, with one exception worth keeping: avatars of a commit's author and
committer, which Gravatar serves for the hash of an email address.

Loading those avatars unconditionally discloses a hash of every contributor's email address, and
the reader's IP address and timing, to a third party the developer never chose, on every opening
of a repository. A hash of an email address is not anonymous: for any address one can guess, one
can confirm it. The disclosure is the problem, not the feature: avatars are a piece of polish
worth keeping once the developer has agreed to it.

The desktop application shows the web interface in a WebView
([ADR-0018](0018-tauri-desktop-host.md)), and a Tauri configuration can grant it room the
interface never uses: `"csp": null` lets the WebView load and connect anywhere, an enabled asset
protocol with the scope `["**"]` lets it read any path through `asset://`, and
`withGlobalTauri: true` injects the whole Tauri API into `window`. The interface reaches Tauri
only through ES imports (`@tauri-apps/api/core`, `@tauri-apps/api/event`) and uses neither
`convertFileSrc` nor `asset://` nor `window.__TAURI__`. A repository's content is untrusted
(Threats), and commit messages, ref names, and diffs are drawn in that WebView.

## Decision

We will keep the front ends **local by default and grant the WebView nothing it does not use**.

1. **Avatars are opt-in, default off.** A toggle, "Load avatars from Gravatar", decides whether
   the web interface renders avatar `<img>` elements at all; while it is off, no avatar URL is
   dereferenced and opening a repository makes no network request. The setting is kept in
   `localStorage`, like the theme. Initials are the default and the fallback. The avatar hash is
   **SHA-256** of the trimmed, lowercased address, which Gravatar accepts, computed through `sha2`.
2. **A restrictive Content-Security-Policy** replaces `null`, permitting the bundled interface's
   own scripts, styles, and inline theme bootstrap, the IPC to the host, and exactly one external
   origin: `img-src https://www.gravatar.com`, the one grant the opt-in feature needs.
3. **The asset protocol is disabled**, since nothing uses it. A feature that needs file access from
   the WebView re-enables it with a scope narrowed to that feature, never `**`.
4. **`withGlobalTauri` is `false`**, since the interface imports the API as modules.

The rule this sets: the WebView is granted a capability only when a present feature needs it, and
the grant is scoped to that need. The terminal application makes no network request of its own at
all; avatars do not exist in a terminal.

**Out of scope:** Git's own network operations to remote repositories, which the developer starts
and which run through their Git setup, which is
[ADR-0035](0035-remotes-through-libgit2-transports.md); the headers and origins of the remote
server, which serves the same web interface to a browser and whose policy is
[ADR-0037](0037-remote-server-websocket-per-tab.md).

## Alternatives considered

- **No Gravatar at all** — the simplest privacy story and no external origin in the CSP; not
  chosen, because the opt-in default gives the same behaviour at rest, a fresh install makes no
  request, while keeping the feature for those who choose it.
- **Gravatar always on, with the disclosure documented** — no work, but a local tool tells a
  third party about contributors who never agreed, on every opening; documenting it obtains no
  consent.
- **Avatar requests proxied through the Rust side** — the WebView's CSP stays free of external
  origins, but the request still leaves the machine, now with an HTTP client in the host; the
  `img-src` grant is narrower than a new network-capable dependency.
- **MD5, Gravatar's older hash** — equally accepted, but a weaker hash and a dependency for it; the
  hash does not change the disclosure either way, consent does.
- **Leaving the asset protocol and the global API on** — costs nothing today and might save a
  configuration change later; but they are live permissions for absent features, granted on
  speculation.

## Sources / Prior art

- Tauri, *Security*: capabilities, CSP, and the asset protocol's scope —
  <https://v2.tauri.app/security/>
- MDN, *Content-Security-Policy* —
  <https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Security-Policy>
- Gravatar, *Avatars*: the hash of the lowercased address, MD5 or SHA-256 —
  <https://docs.gravatar.com/api/avatars/images/>
- OWASP, *Secure Product Design*, least privilege —
  <https://cheatsheetseries.owasp.org/cheatsheets/Secure_Product_Design_Cheat_Sheet.html>
- GitGraph's `docs/adr/0018-local-only-no-egress-hardened-webview.md` on `main`, where this
  configuration runs.

## Consequences

- Positive: a fresh install makes no network request of its own, which a test can observe;
  contributors' email hashes leave the machine only after the developer's explicit consent; a
  strict CSP turns an accidental request, or content from an untrusted repository that tries to
  load or run something, into a visible failure; the WebView loses file-system-wide read access and
  a global API it never used.
- Negative / trade-offs: the CSP permits the Gravatar origin even while the toggle is off,
  accepted because a permitted origin without a rendered `<img>` makes no request and a static
  `tauri.conf.json` cannot express a toggle-dependent policy; developers who turn avatars on accept
  the disclosure, and the toggle's label must say so; the CSP has to grow with the interface, and a
  policy too strict fails at runtime in ways easy to misdiagnose.
- Follow-ups: the specification's Non-Goals or Quality Goals stating that the front ends make no
  network request by default.

## Enforcement

The tests of `src-web/src/hardening.test.ts` cite this ADR with a `Verifies:` marker and read the
shipped `src-tauri/tauri.conf.json`: the CSP is not `null`, it grants exactly one external origin
and only for images, the asset protocol is not enabled, and `withGlobalTauri` is false. A test in
`crates/core` cites it for the avatar hash being SHA-256 over the normalised address.

`src-web/src/avatars.test.ts` cites it for the avatar toggle being off unless the developer
turned it on.
