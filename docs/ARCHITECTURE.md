# Architecture — Ocelli

> The system **as it stands today**: the parts it is built from, what each is responsible for, and
> how they fit together. It holds no rule and no decision. It names the ADR behind each structural
> choice; where it disagrees with an accepted ADR, the ADR is right and this document is stale. It
> is updated in the same change as the structure it describes
> ([ADR-0001](adr/0001-agent-governance-model.md)).
>
> **Last design revision:** 2026-10-04, due after 20 changes. The revision that ran moves the
> date; the number is this project's to set; a sensor counts the changes outside `docs/` since
> the commit that set the date and says when the next is due
> ([ADR-0006](adr/0006-feature-layer.md)).

## Context

What sits outside the system and what crosses its boundary. A diagram earns its place here more
than anywhere else, and none beats one that has stopped being true.

```
 developer ──▶ desktop application (src-tauri + src-web) ─┐
 developer ──▶ terminal application (crates/tui) ─────────┤──▶ ocelli-core ◀── ocelli-git ──▶ repository
                                                          │    (layout, port)   (git2)       on disk
                                                          └─ both depend on ocelli-core and ocelli-git
```

Ocelli reads repositories on the local file system through git2 and writes nothing to them. It
makes no network request: the one exception, Gravatar avatars in the desktop application, is off
until the developer turns it on ([ADR-0023](adr/0023-no-egress-hardened-webview.md)). The web
interface runs only inside the desktop host; the remote server of the specification does not
exist yet.

## Building blocks

The parts the system is made of, each with one responsibility, named in the vocabulary of
[`GLOSSARY.md`](GLOSSARY.md). One level deep, deeper only where the size of a part earns it. The
golden path and the test pattern an agent copies from are named here.

- **Core** (`crates/core`, `ocelli-core`) — the commit graph's model, its layout, and the
  repository port `RepositoryReader`. It depends on no technology and no adapter
  ([ADR-0009](adr/0009-ports-and-adapters.md)); the layout in `graph.rs` is pure functions over
  values ([ADR-0010](adr/0010-functional-core-layout.md)), laid out after Pvigier
  ([ADR-0015](adr/0015-layout-after-pvigier-by-chunk.md)) with edges shaped by type
  ([ADR-0030](adr/0030-edge-shapes-and-tracks-out-of-the-main-line.md)).
- **Git adapter** (`crates/git`, `ocelli-git`) — `GitReader`, the repository port implemented
  with git2: history up to the commit limit with the stash and working-directory nodes, the refs
  the sidebar lists, changed files, and diffs ([ADR-0019](adr/0019-git2-repository-reader.md),
  [ADR-0016](adr/0016-history-loads-in-chunks.md),
  [ADR-0024](adr/0024-synthetic-nodes-in-the-first-chunk.md)).
  It starts no program a repository names (Q-4).
- **Terminal application** (`crates/tui`, `ocelli-cli`) — draws the core's graph with box-drawing
  characters on ratatui, reads through `GitReader` on worker threads, and keeps its view state in
  a file ([ADR-0028](adr/0028-terminal-application-ratatui.md),
  [ADR-0029](adr/0029-cli-state-file.md)).
- **Desktop host** (`src-tauri`, `ocelli`) — the Tauri shell: commands that read through
  `GitReader` on the blocking pool and hand the graph to the web interface as JSON
  ([ADR-0018](adr/0018-tauri-desktop-host.md)).
- **Web interface** (`src-web`) — Svelte and TypeScript: the canvas, the sidebar, the commit
  detail, the diff overlay, search, and themes
  ([ADR-0022](adr/0022-svelte-web-interface.md), [ADR-0017](adr/0017-virtual-rendering.md),
  [ADR-0020](adr/0020-diff-overlay-and-file-list.md),
  [ADR-0021](adr/0021-themes-tokens-and-palettes.md),
  [ADR-0026](adr/0026-repository-sidebar.md)).

**The golden path** is the core's layout behind the git adapter: `crates/core/src/graph.rs`, read
through `crates/git/src/lib.rs`. **The test pattern** is a reference test: a fixed or generated
input and the output the reference implementation produced, compared exactly
(`crates/core/tests/layout_reference.rs`, `crates/git/tests/load_reference.rs`), beside the
structural tests of the shape in `crates/core/tests/structure.rs`.

## How it runs

The few paths worth following end to end, such as a request, a job, or a build, and where state
lives between them. Only what a newcomer would otherwise reconstruct from code.

- **Opening a repository.** A front end asks `GitReader` for the commits, which arrive capped at
  the commit limit with progress reported through a callback, then for the refs; `build_graph`
  lays the commits out, and the front end draws the rows in view. The desktop host runs both reads
  on Tokio's blocking pool and sends the graph and the refs to the web interface as two events;
  the terminal application runs them on a thread of its own and receives them as messages.
- **Selecting a commit.** The front end asks `GitReader` for the changed files, and for the diff
  of a file once one is opened; the working-directory node's sentinel SHA reads the uncommitted
  changes.
- **State between sessions.** The web interface keeps the theme, the avatars toggle, the diff
  layout, and the sidebar in `localStorage`; the terminal application keeps the same in
  `ocelli/cli-state.json` under the user's state directory.
