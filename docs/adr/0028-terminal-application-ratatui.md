# ADR-0028: The terminal application is built on ratatui and draws the core's layout unchanged

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `crates/tui/`, the terminal application, package and binary `ocelli-cli`

## Context

The terminal application is one of Ocelli's three front ends (G-18). Many of the moments the
specification's Problem describes arise in a terminal: over SSH, in a container, in a Dev
Container like this repository's own, where no WebView is available. There the developer falls
back to the `git log --graph` the specification calls unreadable.

The layout is a functional core in `ocelli-core` ([ADR-0009](0009-ports-and-adapters.md),
[ADR-0010](0010-functional-core-layout.md)): rows from the temporal topological sort and lanes
from straight branches ([ADR-0015](0015-layout-after-pvigier-by-chunk.md)), with the same edges.
A terminal application can draw that layout without changing it, and must: the commit graph a
developer sees in the terminal has to be the one they see on the desktop (Strategy).

The interaction follows [guitar](https://github.com/asinglebit/guitar), a Rust terminal Git
client on ratatui, crossterm, and libgit2. Its concepts, from its README:

- a central graph pane with a synthetic row for uncommitted work on top;
- a left sidebar of panes that can be toggled (branches, tags, stashes, worktrees, submodules);
- an inspector on the right (SHA, parents, author, committer, summary, body) and a pane of changed
  files, with a viewer for diffs;
- vim-style navigation: `j`/`k` to move, `h`/`l` to widen or narrow scope, `Tab`/`Shift+Tab` and
  `Ctrl+h/j/k/l` to move between panes, `Enter` to open, `Esc` to go back;
- graph navigation: `[`/`]` to the child or parent, `{`/`}` to the previous or next commit with a
  ref, `/` to search;
- `1`–`9` to toggle panes; commits coloured by lane; Unicode glyphs with an ASCII fallback.

guitar also brings a layout engine of its own, which is exactly what Ocelli keeps its own
algorithm for. guitar is licensed GPL-3.0-only and Ocelli MIT (Constraints), so its concepts are
adopted and no code is copied.

## Decision

We will build the terminal application as the workspace crate `crates/tui` (package and binary
`ocelli-cli`) on **ratatui 0.30** with the **crossterm 0.29** backend, drawing the commit graph
that `ocelli-core` lays out, unchanged.

1. **The layout is reused, not re-implemented.** `crates/tui` depends on `ocelli-core` and consumes
   its commit graph and `RepoRefs` as plain Rust values; it assigns no row and no lane of its own.
   It depends on `ocelli-git` only to hand the adapter to the port at start, and neither `core` nor
   `ocelli-git` depends on ratatui or crossterm.
2. **Edges map onto box-drawing characters on a grid of cells.** Each lane takes two cells (glyph
   and connector), each row one line. The shapes of
   [ADR-0030](0030-edge-shapes-and-tracks-out-of-the-main-line.md) translate one to one:
   vertical runs `│`, horizontal runs `─`, and the softened corner a rounded box corner
   (`╭ ╮ ╰ ╯` by direction). Where edges meet, the cell shows the matching junction
   (`┴`, `┤`, …); where a horizontal run crosses another lane, the cell shows `┼` in the crossed
   lane's colour, so that lane is never interrupted. An edge's colour is its child's lane colour
   for a branch edge and its parent's for a merge edge, as in the web interface. A commit is `●`, a
   stash node `◆`, the working-directory node `◌`; an ASCII set (`|`, `-`, `+`, `*`) is selected
   by `--ascii` for terminals without box drawing.
3. **Drawing is virtual** ([ADR-0017](0017-virtual-rendering.md)): each frame rasterises only the
   visible rows, and an edge is drawn when its `min_row..=max_row` span intersects them.
4. **Loading never blocks the event loop.** The load through `RepositoryReader` runs on a
   `std::thread` ([ADR-0019](0019-git2-repository-reader.md)); the port's progress callback
   forwards to the event loop, which drains it without blocking and needs no async runtime.
5. **Panes and keys follow guitar**: commit graph in the centre, the repository sidebar on the left
   ([ADR-0026](0026-repository-sidebar.md),
   [ADR-0027](0027-submodules-and-repository-switching.md)), inspector and changed files on the
   right, and the diff over the graph pane ([ADR-0020](0020-diff-overlay-and-file-list.md)), with
   the keys listed in the Context.
6. **No request of its own** ([ADR-0023](0023-no-egress-hardened-webview.md)): a terminal shows no
   avatars, so the one opt-in exception does not arise.

**Out of scope:** the editing operations (G-12 to G-14 and G-16) and their keys, which
[ADR-0033](0033-writes-what-git-writes.md) and
[ADR-0034](0034-discarding-work-only-after-confirmation.md) decide and which the terminal offers
like every front end (G-18); a non-interactive printer of the commit graph; packaging `ocelli-cli`.

## Alternatives considered

- **Porting guitar's graph engine, or forking guitar** — the fastest route to a rich terminal
  application, but a second layout next to ADR-0015 draws a different commit graph for the same
  repository, and GPL-3.0 code cannot enter an MIT code base.
- **A non-interactive printer like `git log --graph`, with Ocelli's layout** — simple, no event
  loop, but no commit detail, no diff, and no navigation, which G-9 asks for; it remains a cheap
  addition on the same rasteriser.
- **A subcommand of the desktop binary** — the binary links webkit2gtk, so it cannot start on a
  headless machine, the very place the terminal application is for.
- **cursive or tui-realm instead of ratatui** — cursive is retained-mode with its own widget tree,
  suited to forms more than a rasterised graph; tui-realm is a component layer on ratatui that adds
  indirection without a present need. ratatui is the common Rust terminal library, and guitar
  proves it for this task.
- **termion instead of crossterm** — Unix only; crossterm also covers Windows (Constraints).

## Sources / Prior art

- guitar, its README, keys, and module layout — <https://github.com/asinglebit/guitar>; its
  dependencies (`ratatui = "0.30.0"`, `crossterm = "0.29.0"`, `git2 = "0.20.4"`) —
  <https://github.com/asinglebit/guitar/blob/main/Cargo.toml>
- ratatui — <https://ratatui.rs/>
- crossterm — <https://crates.io/crates/crossterm>
- Unicode, *Box Drawing*, U+2500–U+257F, with the arc corners U+256D–U+2570 —
  <https://www.unicode.org/charts/PDF/U2500.pdf>
- tig and lazygit, terminal Git interfaces that draw history with box drawing —
  <https://jonas.github.io/tig/>, <https://github.com/jesseduffield/lazygit>
- GitGraph's `docs/adr/0025-terminal-frontend-ratatui.md` on `main`, where this terminal front end
  runs.

## Consequences

- Positive: Ocelli works over SSH, in containers, and in the Dev Container; the layout gains a
  second consumer, which keeps the boundary of ADR-0009 honest; the crate builds without webkit,
  so it is cheap in CI.
- Negative / trade-offs: two front ends to keep in step; two direct dependencies (ratatui,
  crossterm) and their transitive set in `Cargo.lock`; a cell cannot express the web interface's
  curve radius or line weight, so the terminal draws a coarser picture of the same layout; ref
  chips carry their kind as data, since decorative emoji in a name are double width in most
  terminals.
- Follow-ups: packaging `ocelli-cli` in the release pipeline.

## Enforcement

The tests of `crates/tui/src/ui.rs` and `crates/tui/src/raster.rs` cite this ADR with a
`Verifies:` marker: the graph pane shows ref chips, lanes, SHAs, and messages from the layout
`ocelli-core` computes without recomputing it, a tiny terminal does not panic, and `--ascii`
replaces every box-drawing character. The structural test of ADR-0009 holds the direction of the
dependencies.
