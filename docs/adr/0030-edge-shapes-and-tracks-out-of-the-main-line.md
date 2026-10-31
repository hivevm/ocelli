# ADR-0030: An edge's shape follows its type, and a merge out of the main line takes a track of its own, so no edge runs through a commit

- **Status:** 🟢 accepted
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the edge geometry in `crates/core/src/graph.rs`, `src-web/src/GraphCanvas.svelte`, `crates/tui/src/raster.rs`, and `crates/measure/src/crossings.rs`

## Context

G-4 asks that an edge between lanes is drawn as an orthogonal L-shape whose bend follows its type,
a branch edge bending at its parent's row and a merge edge at its child's row, and that no edge
runs through a commit it does not join. Which of the two segments comes first decides which lane
the vertical run occupies, and so whether it can cross a commit.

A rule keyed on the side the child is on, horizontal first when the child is to the right and
vertical first when it is to the left, silently assumes that branches always go right and merges
always go left. They do not. A merge edge whose child lies to the right of its parent is ordinary,
merging the main branch into a feature branch produces one, and under that rule it runs along the
parent's row and then up the **child's** lane. Nothing reserves that lane between the two rows;
when the merge commit's own first-parent line continues in it, the edge runs straight up through
that line's commits.

The forbidden lanes J(c) of [ADR-0015](0015-layout-after-pvigier-by-chunk.md) keep the
**parent's** lane clear over the row range of its merge edges. They cannot protect the child's
lane.

The main line holds the leftmost lane alone ([ADR-0032](0032-main-line-in-the-leftmost-lane.md)).
J(c) cannot keep that lane clear for a merge out of it: when the main branch is merged into a
feature branch, the merge edge's parent stands in lane 0, and the main line's own commits stand in
lane 0 between the parent and the merge commit, so the merge edge's vertical run would pass
through them. Over 200 generated histories, 398 of 429 merges out of the main line did; run up the
child's lane instead, 299 still would.

## Decision

We will choose an edge's shape by its **type**, not by the side its child is on:

| Edge type | Shape | Bend sits at |
| --- | --- | --- |
| Branch edge | along the parent's row, then up the child's lane | parent's row |
| Merge edge | up the parent's lane, then along the child's row | child's row |
| Merge edge out of the main line | along the parent's row to its track, up the track, then along the child's row | parent's row and child's row |
| Either, in one lane | a single vertical line, no bend | — |

Every vertical run then lies in a lane the layout already keeps clear:

- A **branch edge** runs up the **child's** lane. That lane belongs to the child's branch line,
  which stays active, and therefore unavailable to any other commit, until the parent is placed.
- A **merge edge** runs up the **parent's** lane. J(c) forbids every lane used between the highest
  merge child and the parent, so nothing else occupies it over that range.
- A **merge edge out of the main line** runs up a **track** of its own: the layout gives it, as it
  places the parent, the first lane from 1 on that J(c) leaves free over the edge's rows, and
  counts that lane as used over them, as it would for a commit's line.
- Horizontal runs lie on the parent's row (branch edge, and the first run of a merge out of the
  main line) or the child's row (merge edge). Every row holds exactly one commit, so a horizontal
  run can cross other lanes but never pass through a commit.

Every edge is an **orthogonal L-shape**, two straight segments at a right angle with no diagonal
and no curve that leaves the grid, but for a merge out of the main line, which takes three, a
bend at each end, and the joint is softened by a **short quadratic curve** of
radius half the lane width with its control point on the corner (in the terminal, the rounded box
corners `╭ ╮ ╰ ╯`). A branch always bends at the **bottom** (the parent's row) and a merge always
at the **top** (the child's row), whichever side the child is on. The core routes every
edge, `Edge::path` and `Edge::route` in `crates/core/src/model.rs`, from `Edge::is_branch` and the
edge's track, not by comparing lanes; the canvas and `raster.rs` draw the corners it gives them.

**Out of scope:** routing edges around crossings, and reducing crossings (Q-1); the colours of
edges.

## Alternatives considered

- **Keeping the shapes keyed on side, and reserving the child's lane in the layout** until the
  merge parent is placed — cannot work for the common case: the reserved lane is the merge
  commit's own branch line, whose commits sit exactly where the edge runs; it would also widen the
  graph by holding lanes that are otherwise free.
- **Routing around occupied cells** (channel routing) — avoids every overlap, but needs a routing
  pass with its own complexity and gives up the two-segment shape that makes an edge traceable.
- **Three segments for every merge to the right** (out, up, back in) — avoids the commit, but
  breaks the L-shape G-4 asks for and costs a lane of room where J(c) already keeps the parent's
  lane clear; only a merge out of the main line, whose parent's lane is not the layout's to keep
  clear, takes them.
- **A merge out of the main line riding up lane 0** — the main line stays as it is, but the edge
  runs over the main line's commits, and which of them was merged cannot be read off the graph.
- **Accepting the overlap** — contradicts G-4.

## Sources / Prior art

- Pierre Vigier, *Commit Graph Drawing Algorithms*, where J(c) is the set of lanes "non-empty
  between rows min{d.i} and c.i", protecting the parent's lane only —
  <https://pvigier.github.io/2019/05/06/commit-graph-drawing-algorithms.html>; his reference
  implementation computes it the same way, in `src/renderer/helpers/commit-graph.ts`,
  `computePositions` — <https://github.com/pvigier/gitamine>
- `git log --graph`, where a merge's second-parent line turns at the merge commit's row, the same
  "merges bend at the top" — <https://git-scm.com/docs/git-log>
- GitGraph's `docs/adr/0027-edge-shape-by-edge-type.md` on `main`, where these shapes run, with the
  measurement behind them: over 200 generated histories of 120 commits, side-keyed shapes ran 361
  merge edges to the right through a commit; with type-keyed shapes no edge did over 2 000.
- The count of merges out of the main line above, taken over 200 generated histories of 150
  commits with the main line of ADR-0032.

## Consequences

- Positive: no edge can run through a commit, by construction rather than by luck, since each
  vertical run lies in a lane the layout guards; branches and merges are told apart by where they
  bend, independent of direction; the property test covers every edge.
- Negative / trade-offs: a merge to the right bends at the top, in the parent's lane, which some
  developers will not expect from other tools; a merge edge's vertical run keeps the parent's lane
  free over its range, which can make a graph a lane wider than one drawn by hand; a merge out of
  the main line takes three segments and a lane of room for its track; an edge carries its track,
  which both renderers draw.
- Follow-ups: none.

## Enforcement

The tests of `crates/core/src/graph.rs` and `crates/tui/src/raster.rs` cite this ADR with a
`Verifies:` marker: a branch edge bends at the parent's row and a merge edge at the child's,
whichever side the child is on; a merge to the right clears the child's lane; a merge out of the
main line runs up its track; and property tests over generated histories, with and without a main
line, assert that no edge of any kind runs through a commit.

`src-web/src/edgePath.test.ts` cites it for the canvas rounding the corners the core routes.
