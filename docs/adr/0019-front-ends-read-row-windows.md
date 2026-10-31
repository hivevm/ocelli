# ADR-0019: The core keeps the commit graph, the front ends read windows of rows from it, and the core searches it and computes lineage

- **Status:** ⚪ superseded by ADR-draft-row-window-margin
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the commit graph's store in `crates/core`, the desktop host's commands in `src-tauri`, the web interface's graph, search, and lineage in `src-web`, and the terminal application's reads in `crates/tui`

## Context

A desktop host that sent the whole laid-out graph to the web interface as one JSON event would
have the web interface keep every node in memory, search it, and compute lineage over it, while
the terminal application holds the same graph in its process. With the whole history loading
([ADR-0016](0016-history-loads-in-chunks.md)), that graph grows to a million nodes. Q-3 limits
what the commit graph may add to the desktop application to 150 MB at 100 000 commits, and Q-2
allows no wait above 100 ms while scrolling. A million JavaScript objects in the
WebView meet neither; the drawing already touches only the rows in view
([ADR-0020](0020-virtual-rendering.md)).

## Decision

We will keep the laid-out commit graph in the core, in the process of the front end's host, and
have each front end read only the **window of rows** it draws, with the edges that cross it, while
the **core searches** the graph (G-11) and **computes lineage** (G-10) for every front end.

- The desktop host offers the window, the search, and the lineage as Tauri commands; the web
  interface keeps no more than the rows it draws and asks again as the view scrolls or a chunk
  arrives.
- The terminal application reads the same window, search, and lineage from the core in its own
  process.
- What the developer sees and does is what a front end holding the whole graph would show: the
  rows, the search results, and the lineage highlight are the same.

**Out of scope:** the remote server's protocol, which reads the same core and which
[ADR-0035](0035-remote-server-websocket-per-tab.md) decides (G-18); the caching of windows in the
web interface; the commit detail and the diff, which are read per commit.

## Alternatives considered

- **Send every chunk to the web interface** — fewer changes, but the WebView holds the whole graph,
  and Q-3 and Q-2 are out of reach on a large history.
- **Keep search and lineage in each front end over its window** — no move of code, but a window
  does not know the commits outside it, so neither could find or highlight them.

## Sources / Prior art

- [ADR-0020](0020-virtual-rendering.md), which already ties the cost of drawing to the rows in
  view; this ties the data to them.
- The conventions' entry that a behaviour both front ends offer lives once in the core, and only
  drawing is ported ([`docs/CONVENTIONS.md`](../CONVENTIONS.md)): search and lineage get one
  implementation instead of two.

## Consequences

- Positive: the WebView's memory follows the view, not the history (Q-3); scrolling asks for a
  window of rows, whose cost does not grow with the history (Q-2); search and lineage exist once,
  in the core, for every front end, including the remote server.
- Negative / trade-offs: a scroll in the desktop application is a round trip to the host; the web
  interface reads the graph through commands rather than holding it.
- Follow-ups: none.

## Enforcement

Tests of the core cite this ADR with a `Verifies:` marker: a window returns exactly the rows in its
range and every edge whose span crosses it; search and lineage over the core return what the web
interface's former helpers returned for the same graph, carried over as reference cases. The
measurement of Q-3 keeps the desktop application's memory in check.
