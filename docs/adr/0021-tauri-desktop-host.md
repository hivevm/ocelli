# ADR-0021: Tauri hosts the desktop application

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `src-tauri/` (package `ocelli`), its `tauri.conf.json`, and the commands and events between it and `src-web/`

## Context

The desktop application is one of Ocelli's three front ends (G-18) and runs on Linux, macOS, and
Windows (Constraints). It needs fast, direct access to the local repository, a drawing surface
flexible enough for the commit graph with its overlays, and installers a developer runs without a
separate runtime. Being lighter than an Electron application is half of the problem Ocelli solves:
Q-3 allows showing the commit graph of a repository of 100 000 commits to add less than 150 MB to
the desktop application's memory, its WebView's processes included.

[ADR-0007](0007-toolchain-rust-and-typescript.md) decides the languages: Rust for the core and the
desktop host, TypeScript for the one web interface that the desktop application and the remote
server share. It leaves the desktop host open. The host is what joins the two: it runs the Rust
side, shows the web interface in a window, and carries the calls between them.

## Decision

We will host the desktop application in **Tauri**: its Rust side in `src-tauri` drives `core`
through the repository port ([ADR-0008](0008-ports-and-adapters.md)) and serves the web interface
of `src-web` in the system's WebView, the two speaking through Tauri commands and events.

**Out of scope:** the web interface's framework, which is
[ADR-0023](0023-svelte-web-interface.md); the WebView's permissions, which are
[ADR-0025](0025-no-egress-hardened-webview.md); packaging and releasing the installers, which is
[ADR-0038](0038-tag-triggered-release.md); and the remote server, which serves the same web
interface by a protocol of its own
([ADR-0035](0035-remote-server-websocket-per-tab.md)).

## Alternatives considered

- **Electron** — the same web interface and a much larger ecosystem, but it ships a full Chromium
  with every application: a large download and the resident footprint the specification's Problem
  names as the reason GitKraken is no option.
- **A native Rust GUI toolkit (egui, iced, GTK bindings)** — no WebView and one language
  throughout, but a second interface next to the web interface the remote server needs, which is
  the duplication [ADR-0007](0007-toolchain-rust-and-typescript.md) avoids; the toolkits are also
  less mature for rich text, diffs, and styling.
- **The remote server on the local machine, opened in the developer's browser** — no desktop host
  at all, but no installable application and no window of its own, and a local port that Q-5's
  authentication has to guard on every desktop.

## Sources / Prior art

- Tauri, *Process model and WebView* — <https://v2.tauri.app/concept/architecture/>: the system's
  WebView instead of a bundled browser.
- Tauri, *Calling Rust from the frontend* and *Calling the frontend from Rust* —
  <https://v2.tauri.app/develop/calling-rust/>
- Electron — <https://www.electronjs.org/>
- GitGraph's `docs/adr/0013-rust-tauri-technology-base.md` on `main`, where this host runs.

## Consequences

- Positive: a native desktop application with no separate runtime and a small binary compared
  with a bundled Chromium, which is what Q-3 asks; the web interface keeps full freedom for drawing
  on a canvas; the Rust side calls `core` directly, with no protocol in between.
- Negative / trade-offs: a serialisation boundary between Rust and TypeScript; the WebView is the
  system's, so drawing can differ across platforms and their versions; Tauri's prerequisites
  (WebView2, webkit2gtk) must be present on developer machines, in the Dev Container
  ([ADR-0009](0009-dev-container.md)), and in CI; the web interface reaches the Rust side
  through Tauri here and through the remote server elsewhere, so it calls both through one
  interface of its own; whether the system WebView meets Q-3 is measured on Linux only.
- Follow-ups: none open; the interface through which the web interface reaches either host is
  the transport of [ADR-0035](0035-remote-server-websocket-per-tab.md).

## Enforcement

`scripts/measure-memory.sh` measures the desktop application's footprint against Q-3 on Linux. It
decides the criterion, not the host, so it cites Q-3 alone.

**Not mechanically decidable:** that Tauri is the right host. A test can see that the manifests
name it, which restates the decision instead of deciding it.
