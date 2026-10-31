# ADR-0007: Rust for the core, the terminal application, and the servers, TypeScript on Bun for the one web interface

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** the workspace manifests, `rust-toolchain.toml`, `package.json` and `bun.lock`, `scripts/check-build.sh`, the build job of `.github/workflows/checks.yml`, and the Build, Test & Run section of `README.md`

## Context

The specification asks for a commit graph that stays usable at one million commits (Q-2), for a
desktop application far lighter than an Electron one (Q-3), and for the same commit graph and
operations in three front ends: the desktop application, the web interface served by a remote
server, and the terminal application (G-18), on Linux, macOS, and Windows.

GitGraph, the viewer this repository built before Ocelli, ran on Rust with a TypeScript
interface in Tauri, on Bun, with a terminal front end on ratatui sharing the Rust layout. It
showed that the layout runs in Rust and that one Rust core serves two front ends, and its code
was carried onto Ocelli's architecture, where its layout is the core's today.

Two front ends draw in a browser engine: the desktop application in the system's WebView, the
web interface in the developer's browser. One web interface can serve both; a second one is the
duplication G-18 would have to keep in step.

## Decision

We will write the core, the terminal application, the remote server, and the desktop host in
**Rust** (stable, pinned by `rust-toolchain.toml`, one Cargo workspace), and the one web interface
shared by the desktop application and the remote server in **TypeScript**, with **Bun** as its
package manager and task runner.

- **Commands.** Build: `cargo build --workspace --locked` and `bun run --cwd src-web build`.
  Test: `cargo test --workspace --locked` and `bun test src-web`. Lint: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `bun run --cwd src-web
  lint`, which runs svelte-check and ESLint. `bun install --frozen-lockfile` installs; `bun.lock`
  and `Cargo.lock` are committed.
- **One script.** `scripts/check-build.sh` runs them; `scripts/check-all.sh` invokes it with a
  `run` line, and the `build` job of `checks.yml` runs it after setting up the toolchain, on
  Linux, macOS, and Windows, so that code gated to one platform is compiled where it runs.
- **Sensors**, the linters' own rules first:

  | Kind | Tool | Where |
  |---|---|---|
  | Behaviour | `cargo test`, `bun test`, the `Verifies:` markers ([ADR-0002](0002-decisions-verified-by-tests.md)) | every push |
  | Size and complexity | ESLint `complexity`, `max-lines-per-function`; none for Rust | every push |
  | Duplication | none yet: the design revision adds jscpd when it finds logic written twice | — |
  | Dead code | rustc `dead_code` as an error through `-D warnings` | every push |
  | Skipped tests | `scripts/sensor-tests-kept.sh` (`#[ignore]`, `.skip`) | every push |
  | Dependency direction | the structural test of the shape's ADR | every push |
  | Mutation testing | cargo-mutants over the git adapter's guards, `scripts/measure-mutants.sh` | weekly |

**Out of scope:** the desktop host, which is [ADR-0021](0021-tauri-desktop-host.md); the web
interface's framework, which is [ADR-0023](0023-svelte-web-interface.md); the terminal library,
which is [ADR-0029](0029-terminal-application-ratatui.md); the Git library, which is
[ADR-0017](0017-git2-repository-reader.md); the remote server's protocol, which is
[ADR-0035](0035-remote-server-websocket-per-tab.md); a second package manager, which there is
none of.

## Alternatives considered

- **Rust only** — the interface in egui, or in Leptos or Dioxus compiled to WebAssembly. One
  language, but the interface frameworks are younger, and of GitGraph only the core would have
  carried over.
- **TypeScript only** — Node or Bun for the core as well. Fails Q-2 and Q-3 on its face: the
  layout of a million commits wants native code and memory control.
- **Go, C++, or Kotlin for the core** — viable natively, but none carries GitGraph's layout, and
  none is as close to Tauri as Rust.
- **npm or pnpm instead of Bun** — equally workable; Bun is what GitGraph ran on, one tool for
  install, scripts, and tests.

## Sources / Prior art

- GitGraph's ADRs on the Rust + Tauri base, on Bun, and on the terminal front end sharing the
  layout crate; they did not come into this repository's history, its code did, in the commit
  that carried GitGraph onto Ocelli's architecture.
- Tauri, *Process model and WebView* — <https://v2.tauri.app/concept/architecture/>: the system's
  WebView instead of a bundled browser.
- Bun — <https://bun.sh/docs>: package manager, script runner, and test runner in one.
- Clippy lints — <https://rust-lang.github.io/rust-clippy/master/>; rustc lints —
  <https://doc.rust-lang.org/rustc/lints/listing/warn-by-default.html>.
- Birgitta Böckeler, *Sensors for coding agents* —
  <https://martinfowler.com/articles/sensors-for-coding-agents.html>.

## Consequences

- Positive: one Rust core behind every front end, measured once for Q-2; one web interface for
  two front ends, which halves what G-18 keeps in step; GitGraph's layout code is the core's.
- Negative / trade-offs: two languages and two lockfiles; a three-platform build job costs CI
  minutes on every push; whether a system WebView meets Q-3 is measured on Linux only, by
  `scripts/measure-memory.sh`.
- Follow-ups: none.

## Enforcement

The golden path's tests cite this ADR with a `Verifies:` marker. Check 9 of
[`scripts/check-docs.sh`](../../scripts/check-docs.sh) holds the build job, the `run` line
of `scripts/check-all.sh`, and the README's required checks together.
