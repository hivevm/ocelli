# Architecture Decision Records

This directory contains all Architecture Decision Records (ADRs) for this project. Accepted ADRs are
**binding** for humans and coding agents alike (see [`AGENTS.md`](../../AGENTS.md) in the repository
root). ADRs derive from the specification in [`docs/SPECIFICATION.md`](../SPECIFICATION.md).

## Mechanics

The rules live in three `NUC-` ADRs.
[NUC-decisions-are-proposed-then-accepted](NUC-decisions-are-proposed-then-accepted.md) decides
when an ADR is written, how it is developed, who flips its status, and that an accepted one is
immutable. [NUC-adrs-are-named-by-category](NUC-adrs-are-named-by-category.md) decides how it is
named, cited, superseded, and indexed. [NUC-base-is-fixed](NUC-base-is-fixed.md) decides the fixed
base. This section carries only the mechanics of the record;
[`scripts/check-docs.sh`](../../scripts/check-docs.sh) names in its header which of them it
verifies.

- **File.** Copy [`template.md`](template.md) to `<CAT>-<slug>.md` and head it
  `# <CAT>-<slug>: <title>`, the category from the [list](#categories). The author picks the
  identifier when creating the file, and it never changes or returns
  ([NUC-adrs-are-named-by-category §1](NUC-adrs-are-named-by-category.md)).
- **Wording.** The *Decision* and the *Enforcement* follow the writing rule of
  [`AGENTS.md` §4](../../AGENTS.md#4-working-style).
- **Index.** The tables below list the project's ADRs, grouped by category, and no `NUC-` ADR
  ([NUC-adrs-are-named-by-category §5](NUC-adrs-are-named-by-category.md)). They change in the same
  commit as the ADR they describe, for additions, supersessions, and status flips alike. The status
  is shown via the legend's bullet and the ADR's `Applies to` header is mirrored verbatim in its own
  column. The index routes a reader from a change to the decisions that bind it, so a row says what
  its ADR governs without the file being opened. Two tables keep what a session reads apart from
  what it looks up: an accepted or proposed ADR sits under **Binding**, a superseded or rejected one
  under **Superseded and rejected**, and a status flip moves the row.
- **Identifiers are permanent once on `main`.** A normalization keeps them, and only a merged or
  split ADR takes a new one
  ([NUC-decisions-are-proposed-then-accepted §8](NUC-decisions-are-proposed-then-accepted.md)).
  Between two normalizations, never rename, delete, or merge an ADR there: other ADRs, commits
  (`Implements <CAT>-<slug>`), and code cite it. A superseded ADR keeps its file and its body. The
  superseding ADR's `Supersedes` field names it from the proposal on; its `Status` line flips to
  `⚪ superseded by <CAT>-<slug>` in the change where a human accepts the successor
  ([NUC-adrs-are-named-by-category §4](NUC-adrs-are-named-by-category.md)), its index row moving
  with it.
- **Between normalizations, sprawl is curbed by supersession, never by editing.** One ADR may
  supersede several whose decisions have grown into one. Its `Supersedes` field names each, and
  each flips its status when the superseding ADR is accepted, so the set an agent reads shrinks
  while every identifier and every body stays.
- **A normalization restates the record**
  ([NUC-decisions-are-proposed-then-accepted §8](NUC-decisions-are-proposed-then-accepted.md)).
  - **Start.** The tree has no proposed ADR, no feature spec or ticket under `docs/plan/`, and
    green checks.
  - **Steps.** Three, in this order on one branch, each handed to the human and confirmed
    before the next begins: the restated specification, whose `G-n` and `Q-n` run from 1; the
    supersessions it calls for; and the restated record. The record begins once the human has
    decided each supersession. One pull request lands them, merged by a human.
  - **Identifiers.** An ADR restated alone keeps its identifier; a merged or split one takes a
    new one. The `NUC-` files stay as the release brought them.
  - **Clauses.** Every clause of an old ADR lives on in one new ADR. The hand-off of the
    record lists each clause with the new ADR that carries it.
  - **Header.** A new ADR keeps the status of the ADRs it restates. Its `Deciders` name all of
    theirs, its `Date` is the latest of theirs, and it has no `Supersedes` field. In an
    adoption, an ADR the project had accepted stays accepted. A decision found outside any
    record enters as proposed.
  - **Dead ADRs.** Superseded and rejected ADRs are deleted. What one of them chose, and why it
    did not stand, becomes an entry under *Alternatives considered* in the new ADR that governs
    its topic.
  - **References.** Each step moves its references with a map, old to new, applied by the map
    script of the `normalize` skill. The map stands in the hand-off and the
    commit body, and in no document. A commit older than the normalization cites the identifiers
    it had then. Each commit of a normalization or of the migration that changes an accepted ADR
    carries the trailer `Normalization: <date>`, kept through any later rewrite of the branch: it
    is the one mark by which `scripts/check-adr-text.sh` lets a commit rewrite an accepted ADR.
- **Cite only what exists.** Every citation, `<CAT>-<slug>` or a clause `<CAT>-<slug> §n`, names a
  file already in this directory. An anticipated follow-up is described by topic ("a follow-up ADR
  on session storage"). What counts as a citation, and its form in Markdown, is
  [NUC-adrs-are-named-by-category §6](NUC-adrs-are-named-by-category.md).

## Index

**The base:** the template's `NUC-` ADRs in this directory, each named with its blob in
[`NUC.blobs`](NUC.blobs), as `AGENTS.md` is. They bind like any ADR in force, and a project
extends or narrows a clause in an ADR of its own, never edits one
([NUC-base-is-fixed §1](NUC-base-is-fixed.md)).

### Categories

The category of an identifier says what its decision is about
([NUC-adrs-are-named-by-category §2](NUC-adrs-are-named-by-category.md)). The fixed ones:

- **NUC** — the template's own ADRs, reserved to the template repository.
- **SEC** — security.
- **OPS** — build, release, deployment, and operation.
- **API** — interfaces the system offers and consumes, adapters among them.

The project's own follow here, each in the same form with one line on what it holds:

- **ARCH** — the shape of the code and the languages and frameworks it is built on.
- **DEV** — how the project is developed: the harness and the development environment.
- **GIT** — reading and writing repositories through the git adapter.
- **GRAPH** — the commit graph: its layout, its loading, and the rows a front end reads.
- **VIEW** — what the front ends show and how a developer works them.

**Status legend:** 🟢 accepted · 🟡 proposed · 🔴 rejected · ⚪ superseded

### Binding

The decisions in force: accepted, and proposed ones that bind the work implementing them. This
is the table a session reads.

| ADR | Title | Applies to | Status |
|-----|-------|------------|--------|
| [ARCH-rust-core-typescript-web](ARCH-rust-core-typescript-web.md) | Rust for the core, the terminal application, and the servers, TypeScript on Bun for the one web interface | the workspace manifests, `rust-toolchain.toml`, `package.json` and `bun.lock`, `scripts/check-build.sh`, the build job of `.github/workflows/checks.yml`, and the Build, Test & Run section of `README.md` | 🟢 |
| [ARCH-ports-and-adapters](ARCH-ports-and-adapters.md) | The code is structured as ports and adapters, with a core crate that depends on no technology | every crate of the Cargo workspace and the dependencies between them | 🟢 |
| [ARCH-tauri-desktop-host](ARCH-tauri-desktop-host.md) | Tauri hosts the desktop application | `src-tauri/` (package `ocelli`), its `tauri.conf.json`, and the commands and events between it and `src-web/` | 🟢 |
| [ARCH-svelte-web-interface](ARCH-svelte-web-interface.md) | The web interface is built with Svelte 5, and the commit graph stays imperative canvas drawing | the component framework and the build of `src-web/` | 🟢 |
| [ARCH-ratatui-terminal-application](ARCH-ratatui-terminal-application.md) | The terminal application is built on ratatui and draws the core's layout unchanged | `crates/tui/`, the terminal application, package and binary `ocelli-cli` | 🟢 |
| [API-host-answers-core-decisions](API-host-answers-core-decisions.md) | The desktop host answers what the core decides from the refs, and the web interface restates none of it | the `load-refs` event and the `select_row` and `selection` answers of `crates/commands/src/` and `src-tauri/src/`, the transfers in `crates/core/src/transfer.rs`, the history operations in `crates/core/src/operations.rs`, the toolbar and commit detail of `src-web/src/`, and the fixtures in `crates/core/tests/fixtures/` | 🟢 |
| [API-websocket-per-tab](API-websocket-per-tab.md) | The remote server carries the desktop host's commands and events over one WebSocket per tab, and writes only when started with `--allow-writes`, as the user it runs as, one write per repository at a time | the remote server's crate and binary `ocelli-server`, the command handling the desktop host in `src-tauri/` and the remote server share in `crates/commands`, the transport through which `src-web/` reaches either host, the fixtures of the wire format in `crates/core/tests/fixtures/` and `crates/server/tests/fixtures/`, the `--allow-writes` switch, how `crates/server/src/socket.rs` answers `write_discards`, `perform_write`, and `cancel_write` and what it does when a tab closes, the turns of a repository's writes and the registry of open repositories in `crates/server/src/turns.rs`, and the tests of Q-4, Q-5, Q-7, and Q-6 against the remote server | 🟢 |
| [DEV-agent-implements-session-reviews](DEV-agent-implements-session-reviews.md) | One agent implements, a fresh session reviews, and the maintainer reads every change | every change to the repository, who makes it, and who reviews it | 🟢 |
| [DEV-container-from-dockerfile](DEV-container-from-dockerfile.md) | The Dev Container is built from a Dockerfile with Rust and Bun as Features, and runs no container engine | `.devcontainer/Dockerfile`, the Features and the image of `.devcontainer/devcontainer.json`, `scripts/check-devcontainer-toolchain.sh`, and the tests that need a service or a container | 🟢 |
| [GIT-git2-reads-repositories](GIT-git2-reads-repositories.md) | git2 reads repositories behind the repository port, off the thread that draws | `crates/git/` (package `ocelli-git`), the `RepositoryReader` port in `crates/core/src/port.rs`, and every call of it from a front end | 🟢 |
| [GIT-writes-what-git-writes](GIT-writes-what-git-writes.md) | Ocelli writes through libgit2 what `git` writes for the same command, refusing first where `git` would or an ADR departs, and running no program | the `RepositoryWriter` port in `crates/core/src/port.rs`, the writes of `crates/git/src/` in `write.rs`, `guard.rs`, `checkout.rs`, `workdir.rs`, `message.rs`, `operations.rs`, `conflicts.rs`, and `stash.rs`, the working directory a pull's fast-forward writes from `remote.rs`, the reading of a stopped operation in `refs.rs`, every call of the writer from a front end and how a front end shows a stopped operation, `crates/git/tests/git_parity.rs`, and the tests of Q-4 | 🟢 |
| [GIT-conflict-labels-path-safe](GIT-conflict-labels-path-safe.md) | A conflict's sides are labelled as `git` labels them, but for what would let a label spell a path | the labels every write that can stop on a conflict gives libgit2 in `crates/git/src/workdir.rs`, the label calls `crates/git/clippy.toml` disallows elsewhere, and the label tests of `crates/git/tests/history.rs` and `crates/git/tests/write.rs` | 🟢 |
| [GIT-discard-only-after-confirmation](GIT-discard-only-after-confirmation.md) | An operation that discards work runs only after the developer confirms it, the core naming which do | the writes of the `RepositoryWriter` port in `crates/core/src/port.rs`, and how every front end calls them | 🟢 |
| [GIT-remotes-through-libgit2-transports](GIT-remotes-through-libgit2-transports.md) | Ocelli reaches remotes through libgit2's own transports with no proxy, credentials only from the developer's setup for a host both parsers read alike, no revoked SSH host key, and what a push took read without the remote's words | fetching, pulling, and pushing in `crates/git/`, how a push learns what the remote took, the `git2` features in `crates/git/Cargo.toml`, how the front ends offer, stop, and reload after a transfer, and the tests of Q-4 | 🟢 |
| [GRAPH-layout-pure-functions-by-chunk](GRAPH-layout-pure-functions-by-chunk.md) | The commit graph is laid out by pure functions over values after Pvigier, chunk by chunk, each chunk below the ones before, in an index arena kept for the load | the layout module of the `core` crate, `crates/core/src/graph.rs`, as a whole, and in it the row and lane assignment, the node storage, and every reference between nodes, and the tests that verify them | 🟢 |
| [GRAPH-main-line-leftmost-lane](GRAPH-main-line-leftmost-lane.md) | The main line holds the leftmost lane alone, the first-parent chain of the main branch | the lane assignment in `crates/core/src/graph.rs`, the choice of the main line in `crates/git`, the load in `crates/core/src/load.rs`, and the Q-1 measurement | 🟢 |
| [GRAPH-edges-take-own-tracks](GRAPH-edges-take-own-tracks.md) | An edge's shape follows its type, and a merge out of the main line takes a track of its own, so no edge runs through a commit | the edge geometry in `crates/core/src/graph.rs`, `src-web/src/GraphCanvas.svelte`, `crates/tui/src/raster.rs`, and `crates/measure/src/crossings.rs` | 🟢 |
| [GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md) | The history loads in chunks of 3 000 commits, the newest first, until all of it is loaded | the revwalk in `crates/git/src/git.rs`, the repository port in `crates/core/src/port.rs`, the chunk size in `crates/core/src/model.rs`, and every front end's load | 🟢 |
| [GRAPH-synthetic-nodes-first-chunk](GRAPH-synthetic-nodes-first-chunk.md) | Stashes and uncommitted changes appear as synthetic nodes in the first chunk of the commit graph | the node kinds and `WORKDIR_SHA` in `crates/core/src/model.rs`, the reading of stashes and the working directory in `crates/git/src/git.rs`, and how the front ends draw them | 🟢 |
| [GRAPH-core-serves-row-windows](GRAPH-core-serves-row-windows.md) | The core keeps the commit graph, the front ends read windows of rows from it reaching one screen beyond the view and draw only the rows in view, and the core searches it and computes lineage | the commit graph's store in `crates/core`, the desktop host's commands in `src-tauri`, the web interface's graph, search, and lineage in `src-web`, the terminal application's reads in `crates/tui`, and the drawing loops of `src-web/src/GraphCanvas.svelte` and `crates/tui/src/raster.rs` | 🟢 |
| [OPS-tag-builds-draft-release](OPS-tag-builds-draft-release.md) | A pushed version tag builds the installers and a Flatpak into a draft release | `.github/workflows/release.yml` and `flatpak/` | 🟢 |
| [OPS-openssl-vendored-on-macos](OPS-openssl-vendored-on-macos.md) | On macOS, Ocelli builds OpenSSL into its binaries and links no library outside the system | the OpenSSL that `crates/git/Cargo.toml` links on macOS, `Cargo.lock`, and the macOS linkage check of the `build` job in `.github/workflows/checks.yml` | 🟢 |
| [SEC-actions-pinned-to-sha](SEC-actions-pinned-to-sha.md) | GitHub Actions are pinned to a full commit SHA and kept current by Dependabot | every `uses:` reference in `.github/workflows/`, and `.github/dependabot.yml` | 🟢 |
| [SEC-secret-scanning-push-protection](SEC-secret-scanning-push-protection.md) | Secrets are guarded by GitHub's secret scanning and push protection, with no scanner of the project's own | every secret the project's CI uses, and `SECURITY.md` | 🟢 |
| [SEC-no-egress-hardened-webview](SEC-no-egress-hardened-webview.md) | The front ends make no network request by default, and the WebView is granted only what it uses | `src-tauri/tauri.conf.json`, the avatar toggle, and every outbound request the web interface and the terminal application can make | 🟢 |
| [SEC-localhost-tls-session-cookie](SEC-localhost-tls-session-cookie.md) | The remote server listens on localhost by default and on a network only over TLS, and admits a browser by a token per start exchanged for a session cookie, from its own origin only | the listening address, TLS, certificate, authentication, session cookie, Origin and Host checks, and rate limit of `ocelli-server`, and the tests of Q-5 | 🟢 |
| [SEC-server-root-confinement](SEC-server-root-confinement.md) | The git adapter, confined to the server root, opens nothing whose files lie outside it, and the merge editor's reasons alone of its refusals reach the browser | every path a browser names to `ocelli-server`, the confinement of the git adapter in `crates/git/src/confine.rs` and the reads it changes in `refs.rs` and `git.rs`, the server root and its folder listing in `crates/server/src/root.rs`, the check `Host::guarded` in `crates/commands` makes before every command, the merge editor's `NotOpened` reason in `crates/core/src/merge.rs` and the `HostError` `crates/commands` makes of it, what `crates/server/src/socket.rs` lets leave the server, and the sidebar's submodule, worktree, and parent rows it serves | 🟢 |
| [VIEW-own-merge-editor](VIEW-own-merge-editor.md) | A conflicted file's text is resolved in a merge editor of Ocelli's own, one column of blocks the core finds, whose saved result the writer takes as the developer's edit | the blocks the core finds in a conflicted file in `crates/core/src/merge.rs`, the merge editor's read and save in `crates/core/src/port.rs` and `crates/git/src/conflicts.rs`, the merge editor of the web interface in `src-web/src/MergeEditor.svelte`, `src-web/src/mergeEditor.ts`, `src-web/src/mergeBlocks.ts`, and `src-web/src/App.svelte`, the merge editor of the terminal application in `crates/tui/src/merge.rs`, `crates/tui/src/app.rs`, and `crates/tui/src/ui.rs`, the fixture `crates/core/tests/fixtures/merge.json`, the tests of `crates/git/tests/merge_editor.rs` and `crates/server/tests/merge_editor.rs`, and the dependencies on CodeMirror and `tui-textarea-2` in `src-web/package.json` and `crates/tui/Cargo.toml` | 🟢 |
| [VIEW-cli-state-file](VIEW-cli-state-file.md) | The terminal application remembers its view state in one JSON file in the user's state directory | `crates/tui/src/state.rs` and the state file it writes | 🟢 |
| [VIEW-themes-tokens-and-palettes](VIEW-themes-tokens-and-palettes.md) | Dark and light themes come from CSS variables, a canvas palette, and a terminal palette | `src-web/src/theme/`, the colours of every Svelte component, and the canvas and terminal palettes | 🟢 |
| [VIEW-diff-overlay-and-file-list](VIEW-diff-overlay-and-file-list.md) | A diff covers only the commit graph, and the changed files show as a tree or a flat list | the diff overlay and the changed-files list in `src-web/src/` and `crates/tui/src/` | 🟢 |
| [VIEW-repository-sidebar](VIEW-repository-sidebar.md) | A repository sidebar lists branches, remotes, every worktree, tags, and submodules under one filter, selecting a branch or tag reveals its commit, and selecting a worktree or submodule switches the viewed repository | `src-web/src/RepoSidebar.svelte`, `src-web/src/refTree.ts`, `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, `crates/tui/src/sidebar.rs`, the sidebar's submodule, worktree, and parent rows, and the flow that switches the viewed repository | 🟢 |

### Superseded and rejected

The record of what was decided against or replaced: read one when a change touches what it
governed, to learn why the current decision stands. A row moves here in the change that
flips its status.

None yet.
