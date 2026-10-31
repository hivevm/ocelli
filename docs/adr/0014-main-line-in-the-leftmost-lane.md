# ADR-0014: The main line holds the leftmost lane alone, the first-parent chain of the main branch

- **Status:** 🟢 accepted
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the lane assignment in `crates/core/src/graph.rs`, the choice of the main line in `crates/git`, the load in `crates/core/src/load.rs`, and the Q-1 measurement

## Context

G-3 asks that the main line is drawn as one straight line down the leftmost lane, with nothing else
in that lane. The layout of [ADR-0013](0013-layout-after-pvigier-by-chunk.md) leaves it out of
scope: Vigier's straight branches give every branch one lane, but which lane falls out of the order
rows are processed in, so the main branch lands wherever the newest commit's line happens to stand,
and moves from repository to repository. A developer reads a history against its main line; finding
it in a different place each time costs the glance the graph exists to save.

The maintainer decided what the main line is and where it stands: the main branch, the local
branch `origin/HEAD` names, else `main`, else `master`, and without any of them the checked-out
commit; its first-parent chain, as `git log --first-parent` follows it, so branches merged into it
keep lanes of their own; and the leftmost lane belongs to it alone over the whole height of the
graph, so the left edge always reads as the main line. A remote branch alone does not make a main
line. The maintainer accepts that the layout no longer equals GitGraph's where a main line exists,
and that the Q-1 records are measured anew.

The layout is a pure function over values ([ADR-0012](0012-functional-core-layout.md)): which
branch is the main branch is a question about the repository, so it is answered where the repository
is read and reaches the layout as a value. The history loads in chunks in commit-date order
([ADR-0016](0016-history-loads-in-chunks.md)), so the main line is found as it arrives: its tip,
then each commit that is the first parent of a commit on it.

## Decision

We will keep the leftmost lane for the main line alone, the first-parent chain from the tip of the
repository's main branch, which the git adapter names and the layout takes as a value, and lay
everything else out to its right; a history without a main line is laid out as
[ADR-0013](0013-layout-after-pvigier-by-chunk.md) lays it out.

- **The main line's tip.** The git adapter names it with the history: the commit of the local
  branch `origin/HEAD` points to by name, else of `main`, else of `master`, else `HEAD`; none when
  `HEAD` does not resolve either.
- **The chain.** A commit is on the main line when it is the tip or the first parent, in commit
  order, of a commit on it; the layout follows the chain as the chunks arrive. A first parent laid
  out before its child, a late child's parent, ends the chain there.
- **The lane.** Every commit on the main line takes lane 0, and lane 0 is forbidden to every other
  commit and kept clear of other lines over the whole height, above the tip included. The other
  phases of [ADR-0013](0013-layout-after-pvigier-by-chunk.md) run unchanged on the lanes from 1 on.
- **No main line.** Without a tip the lanes are assigned exactly as
  [ADR-0013](0013-layout-after-pvigier-by-chunk.md) assigns them, so the reference tests that hold
  the layout to GitGraph's stand; a tip the walk does not reach leaves lane 0 empty.

**Out of scope:** choosing the main line in a front end; drawing the main line differently from
other lines; which commits the front ends show first ([ADR-0016](0016-history-loads-in-chunks.md)).

## Alternatives considered

- **The checked-out branch as the main line** — follows what the developer works on, but the left
  lane changes meaning with every checkout.
- **A main line the developer chooses** — the most flexible, but view state to keep and a layout to
  redo on a change, for a question the main branch answers.
- **Every commit reachable from the main branch in lane 0** — but merged branches would fold
  into the left lane, which then is no longer one straight line.
- **The left lane free above the main line's tip** — more compact, but the left lane changes its
  meaning at the tip.
- **The remote's main branch where no local one exists** — closer to what a mirror means by
  main, but the maintainer chose the checked-out commit there.
- **Leaving the lane to Vigier's order and only sorting lanes afterwards** — keeps the phases
  untouched, but moving lanes after the fact breaks the straight-branch guarantee and moves lanes
  already shown when a chunk arrives.

## Sources / Prior art

- `git log --first-parent` and the first-parent view of a history —
  <https://git-scm.com/docs/git-log>
- `origin/HEAD` as the branch a clone treats as its main one, set by `git clone` and
  `git remote set-head` —
  <https://git-scm.com/docs/git-remote>
- Pierre Vigier, *Commit Graph Drawing Algorithms*, the straight-branch phase this decision
  reserves a lane in — <https://pvigier.github.io/2019/05/06/commit-graph-drawing-algorithms.html>
- [ADR-0013](0013-layout-after-pvigier-by-chunk.md), whose lane phase this decision constrains.

## Consequences

- Positive: the main line stands in the same place in every repository and every front end (G-3);
  the left edge of the graph always reads as the main line; histories without one are laid out as
  [ADR-0013](0013-layout-after-pvigier-by-chunk.md) lays them out.
- Negative / trade-offs: where a main line exists the layout differs from GitGraph's and may draw
  more crossings, which Q-1 records anew; a merge out of the main line needs a track of its own
  ([ADR-0015](0015-edge-shapes-and-tracks-out-of-the-main-line.md)); an edge to or from a late
  child of the main line, where a clock ran wrong, runs along lane 0; a lane is held for the main
  line even where it has not started yet; the git adapter reads which branch is the main branch
  with every load.
- Follow-ups: none.

## Enforcement

The tests of `crates/core/src/graph.rs` cite this ADR with a `Verifies:` marker: with a main line,
every commit of its first-parent chain stands in lane 0 and no other commit does, across chunks;
without one, the layout equals the layout of [ADR-0013](0013-layout-after-pvigier-by-chunk.md),
which the reference test holds; and every edge stays clear of the commits it does not join. The
tests of the git adapter cite it for the choice of the tip: `origin/HEAD`'s branch first, then
`main`, then `master`, then `HEAD`. The Q-1 measurement cites it for measuring the layout with the
main line.
