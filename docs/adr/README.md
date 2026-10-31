# Architecture Decision Records

This directory contains all Architecture Decision Records (ADRs) for this project. Accepted ADRs are
**binding** for humans and coding agents alike (see [`AGENTS.md`](../../AGENTS.md) in the repository
root). ADRs derive from the specification in [`docs/SPECIFICATION.md`](../SPECIFICATION.md).

## Mechanics

The rules live in [`AGENTS.md` §3](../../AGENTS.md#3-adr-rules): when an ADR is written, how it
is developed, who flips its status, that an accepted one is immutable and changed only by
supersession. This section carries only the mechanics of the record;
[`scripts/check-docs.sh`](../../scripts/check-docs.sh) names in its header which of them it
verifies.

- **File.** Copy [`template.md`](template.md) to `draft-<slug>.md`, the slug in lower case letters
  and digits joined by hyphens. Until it reaches `main` the ADR is a draft: its heading and every
  reference name it `ADR-draft-<slug>`, its index row links it as `draft-<slug>`, and no number
  is taken ([ADR-0002](0002-decisions-verified-by-tests.md)). On the branch whose pull request
  brings it to `main`, once that branch has taken up the current `main`,
  [`scripts/number-adrs.sh`](../../scripts/number-adrs.sh) gives it the next free number. A
  numbered ADR is `NNNN-short-title.md`, its `# ADR-NNNN` heading matches the filename, and the
  numbers run `0001..N` without gaps.
- **Index.** The tables below change in the same commit as the ADR they describe, for
  additions, supersessions, and status flips alike. The status is shown via the legend's bullet
  and the ADR's `Applies to` header is mirrored verbatim in its own column. The index routes a
  reader from a change to the decisions that bind it, so a row says what its ADR governs without
  the file being opened. Two tables keep what a session reads apart from what it looks up: an
  accepted or proposed ADR sits under **Binding**, a superseded or rejected one under
  **Superseded and rejected**, and a status flip moves the row.
- **Numbers are permanent once on `main`, until a normalization.** Between two normalizations,
  never renumber, delete, or merge an ADR there: other ADRs, commits (`Implements ADR-NNNN`), and
  code may reference a number. No draft reaches `main`. A branch that already carries a numbered
  proposed ADR whose number another branch brought to `main` first gives it the next free number
  before it merges, in every reference its own branch makes to it. Until a normalization, a
  superseded ADR keeps its file and its body. The superseding ADR's `Supersedes` field names it
  from the proposal on; its `Status` line stays as it is while the successor is proposed, and
  flips to `⚪ superseded by ADR-NNNN` in the change where a human accepts the successor
  ([`AGENTS.md` §3](../../AGENTS.md#3-adr-rules), rule 4), its index row moving with it.
- **Between normalizations, sprawl is curbed by supersession, never by editing.** One ADR may
  supersede several whose decisions have grown into one. Its `Supersedes` field names each, and
  each flips its status when the superseding ADR is accepted, so the set an agent reads shrinks
  while every number and every body stays.
- **A normalization restates the record** ([`AGENTS.md` §3](../../AGENTS.md#3-adr-rules), rule
  5).
  - **Start.** The tree has no proposed ADR, no feature spec or ticket under `docs/plan/`, and
    green checks.
  - **Steps.** Three, in this order on one branch, each handed to the human and confirmed
    before the next begins: the restated specification, whose `G-n` and `Q-n` run from 1; the
    supersessions it calls for; and the restated record. The record begins once the human has
    decided each supersession. One pull request lands them, merged by a human.
  - **Order.** The record numbers the ADRs in force `0001..N` by what they govern. The document
    set and the ADR process come first, then the founding decisions in the order of the setup,
    then the rest in groups. An inherited ADR takes its place like any other, and may be merged
    with the project's own.
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
    commit body, and in no document. A commit older than the normalization cites the numbers it
    had then.
- **Cite only what exists.** Every `ADR-NNNN` reference names a file already in this directory.
  An anticipated follow-up is described by topic ("a follow-up ADR on session storage"), never by
  a number. In Markdown, cite an ADR as a link to its own file.

## Index

An ADR whose `Deciders` line names the **NUC maintainer** is inherited from the template. It
binds a derived project once its own maintainer adds their name to `Deciders` and its `Status`
line reads `🟢 accepted` ([`AGENTS.md` §3](../../AGENTS.md#3-adr-rules)). Change an inherited
decision by superseding it, never by editing. Once the template setup is done, check 12 of
[`scripts/check-docs.sh`](../../scripts/check-docs.sh) fails on an inherited ADR in force that
names no decider of the project, or that is still proposed.

**Status legend:** 🟢 accepted · 🟡 proposed · 🔴 rejected · ⚪ superseded

### Binding

The decisions in force: accepted, and proposed ones that bind the work implementing them. This
is the table a session reads.

| ADR | Title | Applies to | Status |
|-----|-------|------------|--------|
| [0001](0001-agent-governance-model.md) | The document set: one specification, one ADR record, one `AGENTS.md`, one overview, one glossary, one conventions file | every document that carries rules for humans or agents, `AGENTS.md` among them, the architecture overview, the glossary, the conventions, `scripts/check-docs.sh`, `scripts/sensor-binding-adrs.sh`, and the map script of the `normalize` skill, `.agents/skills/normalize/remap.sh` | 🟢 |
| [0002](0002-decisions-verified-by-tests.md) | Every accepted decision and every success criterion is verified by a test that cites it, and the checks that hold the decisions are bash scripts under `scripts/` | every accepted ADR, the identifiers by which ADRs are cited, the Goals and Quality Goals of `docs/SPECIFICATION.md`, the tests that verify them, the checks and sensors under `scripts/`, and the git hooks under `.githooks/` | 🟢 |
| [0003](0003-procedures-as-skills.md) | The procedures of the rule file are Agent Skills under `.agents/skills/`, each carrying a how and no rule of its own | `.agents/skills/`, the pointers under `.claude/skills/`, and every document that describes how a procedure of `AGENTS.md` is carried out | 🟢 |
| [0004](0004-feature-layer.md) | Work larger than one session is planned in the tree as a feature spec cut into tracer-bullet tickets, files deleted when their work lands, and the design is revised when a spec closes or after a number of changes | every change larger than one agent session, `docs/plan/`, the spec and ticket templates of the `plan-feature` skill, the pull request template, the `Last design revision` line of `docs/ARCHITECTURE.md`, `scripts/sensor-revision-due.sh`, `scripts/sensor-plan-frontier.sh`, and the claim script `.agents/skills/work-tickets/claim.sh` | 🟢 |
| [0005](0005-template-releases.md) | The template is released as SemVer tags on `main`, and every repository names the release it carries | the tags of the template repository, its own ADRs, the **Template** section of `README.md`, check 14 of `scripts/check-docs.sh`, and the pull request that lands a release | 🟢 |
| [0006](0006-harness.md) | One agent implements, a fresh session reviews, and the maintainer reads every change | every change to the repository, who makes it, and who reviews it | 🟢 |
| [0007](0007-toolchain-rust-and-typescript.md) | Rust for the core, the terminal application, and the servers, TypeScript on Bun for the one web interface | the workspace manifests, `rust-toolchain.toml`, `package.json` and `bun.lock`, `scripts/check-build.sh`, the build job of `.github/workflows/checks.yml`, and the Build, Test & Run section of `README.md` | 🟢 |
| [0008](0008-ports-and-adapters.md) | The code is structured as ports and adapters, with a core crate that depends on no technology | every crate of the Cargo workspace and the dependencies between them | 🟢 |
| [0009](0009-dev-container.md) | The Dev Container is built from a Dockerfile with locked Features, and keeps every container engine out of reach | `.devcontainer/`, `.vscode/settings.json`, `scripts/check-devcontainer.sh`, `scripts/check-devcontainer-toolchain.sh`, the tests that need a service or a container, and everything the container pulls in | 🟢 |
| [0010](0010-action-references-sha.md) | GitHub Actions are pinned to a full commit SHA and kept current by Dependabot | every `uses:` reference in `.github/workflows/`, and `.github/dependabot.yml` | 🟢 |
| [0011](0011-secrets-baseline.md) | Secrets are guarded by GitHub's secret scanning and push protection, with no scanner of the project's own | every secret the project's CI uses, and `SECURITY.md` | 🟢 |
| [0012](0012-functional-core-layout.md) | The layout of the commit graph is a functional core of pure functions over values | the layout module of the `core` crate and the tests that verify it | 🟢 |
| [0013](0013-layout-after-pvigier-by-chunk.md) | The commit graph is laid out after Pvigier chunk by chunk, each chunk below the ones before, in an index arena kept for the load | the row and lane assignment, the node storage, and every reference between nodes in `crates/core/src/graph.rs`, and the tests that verify them | 🟢 |
| [0014](0014-main-line-in-the-leftmost-lane.md) | The main line holds the leftmost lane alone, the first-parent chain of the main branch | the lane assignment in `crates/core/src/graph.rs`, the choice of the main line in `crates/git`, the load in `crates/core/src/load.rs`, and the Q-1 measurement | 🟢 |
| [0015](0015-edge-shapes-and-tracks-out-of-the-main-line.md) | An edge's shape follows its type, and a merge out of the main line takes a track of its own, so no edge runs through a commit | the edge geometry in `crates/core/src/graph.rs`, `src-web/src/GraphCanvas.svelte`, `crates/tui/src/raster.rs`, and `crates/measure/src/crossings.rs` | 🟢 |
| [0016](0016-history-loads-in-chunks.md) | The history loads in chunks of 3 000 commits, the newest first, until all of it is loaded | the revwalk in `crates/git/src/git.rs`, the repository port in `crates/core/src/port.rs`, the chunk size in `crates/core/src/model.rs`, and every front end's load | 🟢 |
| [0017](0017-git2-repository-reader.md) | git2 reads repositories behind the repository port, off the thread that draws | `crates/git/` (package `ocelli-git`), the `RepositoryReader` port in `crates/core/src/port.rs`, and every call of it from a front end | 🟢 |
| [0018](0018-synthetic-nodes-in-the-first-chunk.md) | Stashes and uncommitted changes appear as synthetic nodes in the first chunk of the commit graph | the node kinds and `WORKDIR_SHA` in `crates/core/src/model.rs`, the reading of stashes and the working directory in `crates/git/src/git.rs`, and how the front ends draw them | 🟢 |
| [0019](0019-front-ends-read-row-windows.md) | The core keeps the commit graph, the front ends read windows of rows from it, and the core searches it and computes lineage | the commit graph's store in `crates/core`, the desktop host's commands in `src-tauri`, the web interface's graph, search, and lineage in `src-web`, and the terminal application's reads in `crates/tui` | 🟢 |
| [0020](0020-virtual-rendering.md) | The commit graph is drawn for the rows in view only | the drawing loops of `src-web/src/GraphCanvas.svelte` and `crates/tui/src/raster.rs` | 🟢 |
| [0021](0021-tauri-desktop-host.md) | Tauri hosts the desktop application | `src-tauri/` (package `ocelli`), its `tauri.conf.json`, and the commands and events between it and `src-web/` | 🟢 |
| [0022](0022-desktop-host-answers-what-the-core-decides.md) | The desktop host answers what the core decides from the refs, and the web interface restates none of it | the `load-refs` event and the `select_row` and `selection` answers of `crates/commands/src/` and `src-tauri/src/`, the transfers in `crates/core/src/transfer.rs`, the history operations in `crates/core/src/operations.rs`, the toolbar and commit detail of `src-web/src/`, and the fixtures in `crates/core/tests/fixtures/` | 🟢 |
| [0023](0023-svelte-web-interface.md) | The web interface is built with Svelte 5, and the commit graph stays imperative canvas drawing | the component framework and the build of `src-web/` | 🟢 |
| [0024](0024-themes-tokens-and-palettes.md) | Dark and light themes come from CSS variables, a canvas palette, and a terminal palette | `src-web/src/theme/`, the colours of every Svelte component, and the canvas and terminal palettes | 🟢 |
| [0025](0025-no-egress-hardened-webview.md) | The front ends make no network request by default, and the WebView is granted only what it uses | `src-tauri/tauri.conf.json`, the avatar toggle, and every outbound request the web interface and the terminal application can make | 🟢 |
| [0026](0026-diff-overlay-and-file-list.md) | A diff covers only the commit graph, and the changed files show as a tree or a flat list | the diff overlay and the changed-files list in `src-web/src/` and `crates/tui/src/` | 🟢 |
| [0027](0027-repository-sidebar.md) | A repository sidebar lists branches, remotes, worktrees, and tags, and selecting a branch or tag reveals its commit | `src-web/src/RepoSidebar.svelte`, `src-web/src/refTree.ts`, `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, and `crates/tui/src/sidebar.rs` | 🟢 |
| [0028](0028-submodules-and-repository-switching.md) | Submodules appear in the repository sidebar, and the sidebar switches the viewed repository | `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, the sidebar's submodule, worktree, and parent rows, and the flow that switches the viewed repository | 🟢 |
| [0029](0029-terminal-application-ratatui.md) | The terminal application is built on ratatui and draws the core's layout unchanged | `crates/tui/`, the terminal application, package and binary `ocelli-cli` | 🟢 |
| [0030](0030-cli-state-file.md) | The terminal application remembers its view state in one JSON file in the user's state directory | `crates/tui/src/state.rs` and the state file it writes | 🟢 |
| [0031](0031-writes-what-git-writes.md) | Ocelli writes through libgit2 what `git` writes for the same command, refusing first where `git` would or an ADR departs, running no program, and resolves a conflicted file's text in a merge editor of its own | the `RepositoryWriter` port in `crates/core/src/port.rs`, the writes of `crates/git/src/` in `write.rs`, `guard.rs`, `checkout.rs`, `workdir.rs`, `message.rs`, `operations.rs`, `conflicts.rs`, and `stash.rs`, the working directory a pull's fast-forward writes from `remote.rs`, the reading of a stopped operation in `refs.rs`, every call of the writer from a front end and how a front end shows a stopped operation, `crates/git/tests/git_parity.rs`, the tests of Q-4, and the merge editor: the blocks the core finds in a conflicted file, the merge editor of the web interface and of the terminal application, and their dependencies on CodeMirror and `tui-textarea-2` | 🟢 |
| [0032](0032-conflict-labels-path-safe.md) | A conflict's sides are labelled as `git` labels them, but for what would let a label spell a path | the labels every write that can stop on a conflict gives libgit2 in `crates/git/src/workdir.rs`, the label calls `crates/git/clippy.toml` disallows elsewhere, and the label tests of `crates/git/tests/history.rs` and `crates/git/tests/write.rs` | 🟢 |
| [0033](0033-discarding-work-only-after-confirmation.md) | An operation that discards work runs only after the developer confirms it, the core naming which do | the writes of the `RepositoryWriter` port in `crates/core/src/port.rs`, and how every front end calls them | 🟢 |
| [0034](0034-remotes-through-libgit2-transports.md) | Ocelli reaches remotes through libgit2's own transports with no proxy, credentials only from the developer's setup for a host both parsers read alike, no revoked SSH host key, and what a push took read without the remote's words | fetching, pulling, and pushing in `crates/git/`, how a push learns what the remote took, the `git2` features in `crates/git/Cargo.toml`, how the front ends offer, stop, and reload after a transfer, and the tests of Q-4 | 🟢 |
| [0035](0035-remote-server-websocket-per-tab.md) | The remote server carries the desktop host's commands and events over one WebSocket per tab, and writes only when started with `--allow-writes`, as the user it runs as, one write per repository at a time | the remote server's crate and binary `ocelli-server`, the command handling the desktop host in `src-tauri/` and the remote server share in `crates/commands`, the transport through which `src-web/` reaches either host, the fixtures of the wire format in `crates/core/tests/fixtures/` and `crates/server/tests/fixtures/`, the `--allow-writes` switch, how `crates/server/src/socket.rs` answers `write_discards`, `perform_write`, and `cancel_write` and what it does when a tab closes, the turns of a repository's writes and the registry of open repositories in `crates/server/src/turns.rs`, and the tests of Q-4, Q-5, Q-7, and Q-6 against the remote server | 🟢 |
| [0036](0036-remote-server-localhost-tls-session-cookie.md) | The remote server listens on localhost by default and on a network only over TLS, and admits a browser by a token per start exchanged for a session cookie, from its own origin only | the listening address, TLS, certificate, authentication, session cookie, Origin and Host checks, and rate limit of `ocelli-server`, and the tests of Q-5 | 🟢 |
| [0037](0037-git-adapter-confined-to-the-server-root.md) | The git adapter, confined to the server root, opens nothing whose files lie outside it | every path a browser names to `ocelli-server`, the confinement of the git adapter in `crates/git/src/confine.rs` and the reads it changes in `refs.rs` and `git.rs`, the server root and its folder listing in `crates/server/src/root.rs`, the check `Host::guarded` in `crates/commands` makes before every command, what `crates/server/src/socket.rs` lets leave the server, and the sidebar's submodule, worktree, and parent rows it serves | 🟢 |
| [0038](0038-tag-triggered-release.md) | A pushed version tag builds the installers and a Flatpak into a draft release | `.github/workflows/release.yml` and `flatpak/` | 🟢 |
| [0039](0039-openssl-vendored-on-macos.md) | On macOS, Ocelli builds OpenSSL into its binaries and links no library outside the system | the OpenSSL that `crates/git/Cargo.toml` links on macOS, `Cargo.lock`, and the macOS linkage check of the `build` job in `.github/workflows/checks.yml` | 🟢 |
| [draft-sidebar-worktrees-filter](draft-sidebar-worktrees-filter.md) | A repository sidebar lists branches, remotes, every worktree, and tags under one filter, and selecting a branch or tag reveals its commit | `src-web/src/RepoSidebar.svelte`, `src-web/src/refTree.ts`, `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, and `crates/tui/src/sidebar.rs` | 🟡 |

### Superseded and rejected

The record of what was decided against or replaced: read one when a change touches what it
governed, to learn why the current decision stands. A row moves here in the change that
flips its status.

| ADR | Title | Applies to | Status |
|-----|-------|------------|--------|
