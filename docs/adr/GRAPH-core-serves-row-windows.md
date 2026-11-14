# GRAPH-core-serves-row-windows: The core keeps the commit graph, the front ends read windows of rows from it reaching one screen beyond the view and draw only the rows in view, and the core searches it and computes lineage

- **Status:** 🟢 accepted
- **Date:** 2026-10-31
- **Deciders:** Markus Brigl
- **Applies to:** the commit graph's store in `crates/core`, the desktop host's commands in `src-tauri`, the web interface's graph, search, and lineage in `src-web`, the terminal application's reads in `crates/tui`, and the drawing loops of `src-web/src/GraphCanvas.svelte` and `crates/tui/src/raster.rs`

## Context

At a chunk of 3 000 rows ([GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md)) and a
row height of
26 px, the commit graph is about 78 000 px tall, while the window shows a few dozen rows; with the
whole history loading, it grows to a million nodes (Q-2) and millions of pixels. G-7 asks that only
the rows in view and the edges that cross them are drawn, Q-2 that scrolling never waits more than
100 ms for the next screen, and Q-3 limits what the commit graph may add to the desktop application
to 150 MB at 100 000 commits.

Two costs grow with the history unless they are tied to the view. Drawing every commit and every
edge on each frame costs `O(n)` per frame and cannot hold 60 frames per second on a dense graph,
although almost all of that work is invisible. And a desktop host that sent the whole laid-out
graph to the web interface as one JSON event would have the web interface keep every node in
memory, search it, and compute lineage over it, while the terminal application holds the same
graph in its process. A million JavaScript objects in the WebView meet neither Q-3 nor Q-2.

A window of exactly the rows in view goes stale with the first row scrolled: every step of a
scroll would wait for a round trip to the host, which Q-2 bounds at 100 ms. The web interface
therefore asks for one screen of rows beyond the view on either side (`src-web/src/rowWindow.ts`).

## Decision

We will keep the laid-out commit graph in the core, in the process of the front end's host, and
have each front end read only the **window of rows** it draws, with the edges that cross it, and
at most one screen of rows beyond the view on either side, and draw only the **rows in view**, an
edge whenever its row span intersects the visible range, while the **core searches** the graph
(G-11) and **computes lineage** (G-10) for every front end.

- **Reading.** The desktop host offers the window, the search, and the lineage as Tauri commands;
  the web interface keeps no more than the rows in view and one screen of rows beyond them on
  either side, and asks again once the view leaves the rows it asked for or a chunk arrives. The
  terminal application reads the same window, search, and lineage from the core in its own
  process. What the developer sees and does is what a front end holding the whole graph would
  show: the rows, the search results, and the lineage highlight are the same.
- **Drawing.** The cost of drawing is tied to the rows in view, not to the size of the history:
  1. Derive the visible row range from the scroll position and the height of the view.
  2. Skip the commits outside that range entirely.
  3. Draw an edge when its span `min_row..=max_row` intersects the visible range, so that an edge
     from a commit in view to one off screen, or between two commits both off screen, is still
     drawn.

  The third step is part of the decision because it is the one easy to get wrong: an edge drawn
  only when one of its ends is in view disappears as soon as both ends scroll away.

**Out of scope:** the remote server's protocol, which reads the same core and which
[API-websocket-per-tab](API-websocket-per-tab.md) decides (G-18); the caching of windows in the
web interface; the commit detail and the diff, which are read per commit; drawing the whole commit
graph off screen, which an export or a screenshot would need; how the web interface and the
terminal application compute the row height.

## Alternatives considered

- **Send every chunk to the web interface** — fewer changes, but the WebView holds the whole graph,
  and Q-3 and Q-2 are out of reach on a large history.
- **No more than the rows in view** — the least memory, and the decision first taken when the
  front ends began to read windows, but every scrolled row is a round trip to the host before it
  is drawn, which Q-2 does not allow on a slow host. It did not stand for that reason, and the
  margin of one screen replaced it.
- **A wider margin, or a cache of windows already read** — fewer round trips, but the memory no
  longer follows the view alone, and a cache is a second copy of the graph to keep in step.
- **Keep search and lineage in each front end over its window** — no move of code, but a window
  does not know the commits outside it, so neither could find or highlight them.
- **Drawing the whole graph every frame** — simplest, but the cost grows with the history and the
  frame budget is spent on pixels nobody sees.
- **Drawing the whole graph once into an off-screen buffer and copying the visible part** —
  constant cost per frame, but a buffer tens of thousands of pixels tall is expensive in memory
  (Q-3) and has to be redrawn on every zoom, theme, or selection change.
- **Tiles drawn once and cached** — a middle ground, but it adds cache invalidation for a problem
  the computation of the visible range already solves.

## Sources / Prior art

- List virtualisation ("windowing"), the standard technique for long scrolling content, as in
  `react-window` — <https://github.com/bvaughn/react-window>; this decision applies it to the
  drawing and to the data alike.
- MDN, *Optimizing canvas* —
  <https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API/Tutorial/Optimizing_canvas>
- The conventions' entry that a behaviour both front ends offer lives once in the core, and only
  drawing is ported ([`docs/CONVENTIONS.md`](../CONVENTIONS.md)): search and lineage get one
  implementation instead of two.
- GitGraph's `docs/adr/0012-virtual-rendering.md` on `main`, where this drawing runs.

## Consequences

- Positive: the cost of a frame is `O(visible rows)` and independent of the history's size;
  scrolling stays smooth; the same rule keeps later overlays cheap; the WebView's memory follows the
  view, three screens of rows at most, not the history (Q-3); scrolling within a screen of the view
  asks the host for nothing, and beyond it asks for a window of rows, whose cost does not grow with
  the history (Q-2); search and lineage exist once, in the core, for every front end, including the
  remote server.
- Negative / trade-offs: edges that leave the view must be clipped correctly, and the
  `min_row`/`max_row` intersection is easy to get subtly wrong; no path draws the complete graph,
  which an export would need; a scroll of more than a screen in the desktop application is a round
  trip to the host; the web interface reads the graph through commands rather than holding it.
- Follow-ups: drawing the whole graph off screen, should an export or screenshot be wanted.

## Enforcement

Tests of the core cite this ADR with a `Verifies:` marker: a window returns exactly the rows in its
range and every edge whose span crosses it; search and lineage over the core return what the web
interface's former helpers returned for the same graph, carried over as reference cases. The web
interface's `src-web/src/rowWindow.test.ts` cites it for the window it asks for: the view and one
screen beyond it on either side, nothing asked while the view stays inside it. The measurement of
Q-3 keeps the desktop application's memory in check.

The tests of `crates/tui/src/raster.rs` cite this ADR with a `Verifies:` marker, the step this
decision calls easy to get wrong included: an edge whose span intersects the window is drawn even
when both its ends sit off screen, a long edge is found by a window far from either end, and a
window past the end of the history is empty rather than wrong.

**Not mechanically decidable:** that the web interface's canvas draws only the visible rows. Its
drawing needs a browser; the shared rule is tested through the terminal rasteriser instead.
