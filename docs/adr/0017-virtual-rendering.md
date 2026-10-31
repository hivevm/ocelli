# ADR-0017: The commit graph is drawn for the rows in view only

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** the drawing loops of `src-web/src/GraphCanvas.svelte` and `crates/tui/src/raster.rs`

## Context

At a chunk of 3 000 rows ([ADR-0016](0016-history-loads-in-chunks.md)) and a row height of
26 px, the commit graph is about 78 000 px tall, while the window shows a few dozen rows; at a
million commits (Q-2) it is millions of pixels. Drawing every commit and every edge on each
frame costs `O(n)` per frame and cannot hold 60 frames per second on a dense graph, although
almost all of that work is invisible.

G-7 asks that only the rows in view and the edges that cross them are drawn, and Q-2 that
scrolling never waits more than 100 ms for the next screen.

## Decision

We will tie the cost of drawing to the **rows in view**, not to the size of the history, drawing
an edge whenever its row span intersects the visible range.

1. Derive the visible row range from the scroll position and the height of the view.
2. Skip the commits outside that range entirely.
3. Draw an edge when its span `min_row..=max_row` intersects the visible range, so that an edge
   from a commit in view to one off screen, or between two commits both off screen, is still
   drawn.

The third step is part of the decision because it is the one easy to get wrong: an edge drawn
only when one of its ends is in view disappears as soon as both ends scroll away.

**Out of scope:** drawing the whole commit graph off screen, which an export or a screenshot
would need; how the web interface and the terminal application compute the row height.

## Alternatives considered

- **Drawing the whole graph every frame** — simplest, but the cost grows with the history and the
  frame budget is spent on pixels nobody sees.
- **Drawing the whole graph once into an off-screen buffer and copying the visible part** —
  constant cost per frame, but a buffer tens of thousands of pixels tall is expensive in memory
  (Q-3) and has to be redrawn on every zoom, theme, or selection change.
- **Tiles drawn once and cached** — a middle ground, but it adds cache invalidation for a problem
  the computation of the visible range already solves.

## Sources / Prior art

- List virtualisation ("windowing"), the standard technique for long scrolling content, as in
  `react-window` — <https://github.com/bvaughn/react-window>
- MDN, *Optimizing canvas* —
  <https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API/Tutorial/Optimizing_canvas>
- GitGraph's `docs/adr/0012-virtual-rendering.md` on `main`, where this drawing runs.

## Consequences

- Positive: the cost of a frame is `O(visible rows)` and independent of the history's size;
  scrolling stays smooth; the same rule keeps later overlays cheap.
- Negative / trade-offs: edges that leave the view must be clipped correctly, and the
  `min_row`/`max_row` intersection is easy to get subtly wrong; no path draws the complete graph,
  which an export would need.
- Follow-ups: drawing the whole graph off screen, should an export or screenshot be wanted.

## Enforcement

The tests of `crates/tui/src/raster.rs` cite this ADR with a `Verifies:` marker, the step this
decision calls easy to get wrong included: an edge whose span intersects the window is drawn even
when both its ends sit off screen, a long edge is found by a window far from either end, and a
window past the end of the history is empty rather than wrong.

**Not mechanically decidable:** that the web interface's canvas draws only the visible rows. Its
drawing needs a browser; the shared rule is tested through the terminal rasteriser instead.
