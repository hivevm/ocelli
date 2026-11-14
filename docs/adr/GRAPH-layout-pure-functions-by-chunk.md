# GRAPH-layout-pure-functions-by-chunk: The commit graph is laid out by pure functions over values after Pvigier, chunk by chunk, each chunk below the ones before, in an index arena kept for the load

- **Status:** 🟢 accepted
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the layout module of the `core` crate, `crates/core/src/graph.rs`, as a whole, and in it the row and lane assignment, the node storage, and every reference between nodes, and the tests that verify them

## Context

The layout is what Ocelli exists for. It places commits in two dimensions: every commit gets a
**row** (its place in time) and a **lane** (its branch), such that the picture reads at a glance.
The specification makes this a success criterion several times over: each branch in one lane
(G-2), the main branch straight on the left (G-3), no edge through a commit it does not join
(G-4), every commit above its parents and ordered by committer date where the topology leaves the
order open (G-5), the same rows and lanes for the same repository state (G-8), no more edge
crossings than recorded for a fixed corpus (Q-1), and fast enough at one million commits (Q-2).
Each of these is a property of a function from commits to rows, lanes, and edges, best tested
exhaustively and on generated histories far larger than any fixture repository. Both axes need a
tie-breaker the developer sees: a commit graph has no unique topological order, and several free
lanes may be available for a branch.

Ports and adapters ([ARCH-ports-and-adapters](ARCH-ports-and-adapters.md)) keeps technologies out of
`core`, but
says nothing against I/O, threads, or a clock inside it. From the catalogue: a functional core
fits where the logic is rich and the effects few, and where a quality goal asks for property-based
tests; it does not fit logic that is mostly effects in a sequence, which the layout is not.

Pierre Vigier's *Commit Graph Drawing Algorithms* answers both tie-breakers in two phases. Vigier
accepts edge crossings in exchange for straight branches; Q-1 holds the layout to the crossings
recorded for its corpus. His rows come from a depth-first traversal over the whole set, which
places a parent right below its child even when it is far older. The history loads in chunks
([GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md)): laid out over the whole
history, older commits
would appear between the newest ones, and the top of the graph would change with every chunk that
arrives. The maintainer decided that the newest commits are laid out as a load of them alone would
be, and that the picture must not shift while older chunks arrive.

The layout walks the graph in both directions, from a commit to its parents and to its children.
Nodes that hold mutable references to each other cannot be expressed directly in Rust; the borrow
checker rejects them. Laid out chunk by chunk, the graph is wired up for as long as the load runs,
since a later chunk joins its commits to children laid out before and a late child to a parent
laid out before. A map keyed by SHA strings then holds an allocation per commit for the whole
load, about 45 MB of the layout's 100 000 commits, and frees them all at its end, where the
allocator cannot hand the memory back (Q-3).

## Decision

We will lay out the commit graph as **pure functions over values** after Pierre Vigier's
algorithm, **one chunk at a time**: each chunk's rows come from the temporal topological sort over
that chunk alone and are placed below the rows of the chunks before it, and its lanes come from the
straight-branch assignment carried on from the chunks before it, so that the first chunk is laid
out exactly as a load of it alone is.

- **Pure functions over values.** Given the commits and their parents, the layout returns the
  lanes, the rows, and the edges, and it performs no I/O, reads no clock, starts no thread, and
  draws no randomness.
- **Rows: temporal topological sort, per chunk.** Rows come from a depth-first traversal whose
  start commits are visited in committer-date order, with rows assigned in post-order, so that the
  topological order is date-consistent as well; `O(n·log n + m)` for `n` commits and `m` parent
  relationships.
  1. Iterate over the chunk's commits sorted by committer date, descending.
  2. For every commit not yet visited, run an iterative depth-first traversal over the chunk that
     assigns row numbers in post-order.
  3. Children receive lower row numbers than their parents; the chunk's rows follow the last row of
     the chunks before it, and row 0 is the newest commit of the first chunk.

  The walk yields a commit after its children unless a child carries an older committer date
  ([GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md)), so a later chunk holds, but
  for such **late
  children**, only ancestors of the chunks before it, and placing a chunk below the ones before
  keeps every parent below its children. A late child is laid out in its chunk like any other
  commit, below its parent, which keeps its place, and is joined to it by an edge that runs up to
  the parent: nothing laid out moves, and only where a clock ran wrong does a commit stand below its
  parent. **Committer date, not author date, is the sort key**: it reflects the shape of the history
  in this repository rather than when a change was first written. **The traversal is iterative**,
  with an explicit work stack, so that a deep history cannot exhaust the call stack.
  A parent that arrives in a later chunk than its child joins the child after the parents laid
  out with it: the child's first line was drawn as the branch it continues, and it stays that, so
  an edge already shown keeps its type and its shape
  ([GRAPH-edges-take-own-tracks](GRAPH-edges-take-own-tracks.md)).
- **Lanes: straight branches, carried across chunks.** Commits are processed top to bottom; the
  active branch lines, the lanes in use, and the rows they were last used at carry on from one
  chunk to the next. Where the history has a main line, it holds lane 0 alone and these steps
  assign the lanes from 1 on ([GRAPH-main-line-leftmost-lane](GRAPH-main-line-leftmost-lane.md)).
  For each
  commit `c`:
  1. Determine its branch children (for which `c` is the first parent) and its merge children.
  2. Compute the **forbidden lanes J(c)** over the row range of its incoming merge edges, from the
     highest merge child's row to `c`'s row: the lanes occupied by a branch line spanning that
     range, and every lane used by a commit or an ending branch line inside it.
  3. Reuse the lane of the leftmost branch child if it is not forbidden; otherwise take the first
     free lane that is not forbidden, appending a new lane if none exists.
  4. Release the lanes of all other branch children that are no longer needed.

  J(c) carries the weight: placing a merge parent in a forbidden lane would make its merge edge
  overlap a line or run through a commit.
- **Storage: an index arena kept for the load.** All nodes of the layout live in one `Vec<Node>`,
  and every reference between them, to a parent or to a child, is a `usize` index into it. The
  arena and one index from a commit's SHA to its node live as long as the load session: the index
  is keyed by a 64-bit hash of the SHA, a SHA whose hash an earlier one holds kept apart, so it
  takes no allocation per commit. Once a chunk is handed over, a node keeps only what wiring the
  chunks after needs; the commit's details go with the chunk.

**Out of scope:** the rest of `core`, which may hold state; keeping the main line in the leftmost
lane, which is [GRAPH-main-line-leftmost-lane](GRAPH-main-line-leftmost-lane.md); reducing edge
crossings below
what the phases yield; the shape of edges, which is
[GRAPH-edges-take-own-tracks](GRAPH-edges-take-own-tracks.md); how chunks are loaded, which is
[GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md); the history the front ends read,
which is
[GRAPH-core-serves-row-windows](GRAPH-core-serves-row-windows.md); and how a front end draws the
result.

## Alternatives considered

- **Ports and adapters alone** — keeps technologies out, but lets the layout read a repository
  incrementally or spawn threads, which makes it testable only through fakes.
- **No split inside `core`** — simplest while the logic is small; the layout is the logic that is
  not small.
- **One traversal over the whole history** — Vigier's layout exactly, but older commits move
  between the newest ones, and the top of the graph changes with every chunk that arrives.
- **Chunks first, one whole layout at the end** — the newest commits at once and Vigier's layout
  exactly in the end, at the price of one visible jump when the load completes.
- **Lay the chunks out again when a late child arrives** — every commit above its parents, but
  rows already shown move, the one thing the chunks are laid out to avoid.
- **A heuristic of our own** — full freedom, but trial-and-error tuning, no reference to compare
  against, and no complexity guarantee.
- **Generic layered layout (Sugiyama, Graphviz `dot`)** — it minimises crossings over layers
  rather than keeping a branch in one lane; the result does not look like a commit graph, and it
  brings a heavy dependency.
- **A plain topological sort (Kahn's algorithm) for the rows** — the order among independent
  commits is arbitrary and unrelated to time, so the graph does not read as a history.
- **Sorting by timestamp alone** — rebases, cherry-picks, and clock skew put parents above their
  children, and edges then point upwards.
- **Author date instead of committer date** — closer to when a change was written, further from
  the shape of the history in this repository.
- **First free lane only, without the forbidden set** — a merge parent can land in a lane another
  branch line spans, and its merge edge then overlaps that line.
- **`Rc<RefCell<Node>>` or `Arc<Mutex<Node>>`** — borrow checks at runtime, an allocation and
  reference counting per node, panics on an aliasing mistake, and locking that one thread over one
  owned structure does not need.
- **`petgraph` or another graph crate** — a proven representation, but a dependency of `core` and
  a general-purpose API for a graph that is built and traversed in passes.
- **`unsafe` raw pointers between nodes** — trades compile-time guarantees for an optimisation
  nothing demands.
- **A map from SHA strings to indices** — the simplest index, but an allocation per commit for the
  whole load, and freed all at once at its end.
- **No index beyond the chunk** — a parent found only in its own chunk or by the children waiting
  for it, the leanest skeleton, but a late child could not find its parent laid out before.
- **One index shared with the history the front ends read** — one index instead of two, but the
  layout runs on the loading thread and the history is read on another, so sharing it means a
  lock on every lookup of either.

## Sources / Prior art

- Gary Bernhardt, *Functional Core, Imperative Shell* —
  <https://www.destroyallsoftware.com/screencasts/catalog/functional-core-imperative-shell>.
- Gary Bernhardt, *Boundaries* — <https://www.destroyallsoftware.com/talks/boundaries>: values at
  the boundary make the core testable without mocks.
- Pierre Vigier, *Commit Graph Drawing Algorithms* —
  <https://pvigier.github.io/2019/05/06/commit-graph-drawing-algorithms.html>: the two phases, and
  the explicit trade of crossings for straight branches.
- K. Sugiyama, S. Tagawa, M. Toda, *Methods for Visual Understanding of Hierarchical System
  Structures*, IEEE Trans. SMC, 1981; Graphviz `dot` — <https://graphviz.org/>
- A. B. Kahn, *Topological sorting of large networks*, Communications of the ACM 5(11), 1962.
- `git log --topo-order` and `--date-order`, Git's own answer to the same tie-breaking question —
  <https://git-scm.com/docs/git-log>
- GitKraken, the product whose graph readability sets the bar — <https://www.gitkraken.com/>
- Niko Matsakis, *Modeling graphs in Rust using vector indices* —
  <https://smallcultfollowing.com/babysteps/blog/2015/04/06/modeling-graphs-in-rust-using-vector-indices/>;
  `petgraph`, which uses the same representation inside — <https://docs.rs/petgraph>;
  *The Rust Programming Language*, chapter 15.6 on reference cycles —
  <https://doc.rust-lang.org/book/ch15-06-reference-cycles.html>
- GitGraph's `docs/adr/0009-pvigier-layout-algorithm.md` and
  `docs/adr/0010-index-arena-instead-of-object-graph.md` on `main`, where this layout and this
  representation run.

## Consequences

- Positive: G-2, G-3, Q-1, and Q-2 are decided by tests on generated histories of any size,
  without a repository; the same input always yields the same picture in every front end; a
  published, proven algorithm instead of trial and error; straight branches by construction (G-2);
  a deterministic, date-consistent order with the newest commits on top (G-5, G-8); the first chunk
  is laid out exactly as a load of it alone, so the reference tests hold for every load; rows and
  lanes already shown never move as further chunks arrive; the layout of a load holds a few
  allocations per chunk rather than several per commit, and hands back all it held at the end of
  the load; `core` gains no dependency and no `unsafe`.
- Negative / trade-offs: incremental loading (Q-2) is the shell's job, feeding the layout in
  chunks it can process as values; parallelism, if the layout needs it, is a separate decision;
  the phases give no protection against edge crossings, so Q-1 is measured, not guaranteed; across
  chunks the layout is an extension of Vigier's, not his: an old parent of a commit near the end of
  a chunk appears in the next chunk rather than directly below it, so a merge edge can reach across
  a chunk boundary; a late child stands below its parent, the one exception G-5 makes, with an edge
  running down to it, which the edge shapes of
  [GRAPH-edges-take-own-tracks](GRAPH-edges-take-own-tracks.md) do not keep clear of commits, the
  one exception G-4 makes; carrying the lane state is state in the core; the algorithm reads as
  index arithmetic, and an index carries no guarantee beyond the bounds check; the layout and the
  history each hold an index of the load's SHAs, one on the loading thread and one where the front
  end reads.
- Follow-ups: reducing crossings should Q-1 fail on the corpus; the glossary gains *forbidden
  lanes* and *branch child*. The chunked feeding of a million commits is
  [GRAPH-history-loads-in-chunks](GRAPH-history-loads-in-chunks.md).

## Enforcement

A structural test in `crates/core/tests/structure.rs` scans the shipped source of the layout
module, `crates/core/src/graph.rs`, and fails on a use of `std::fs`, `std::io`, `std::net`,
`std::process`, `std::thread`, `std::time`, `std::env`, a clock's `now`, or a randomness crate;
it cites this ADR with a `Verifies:` marker. That the shell around the layout stays thin is
review.

The tests of `crates/core/src/graph.rs` and the layout reference test cite this ADR with a
`Verifies:` marker: a linear history is one straight lane, independent tips are ordered by committer
date, the layout is deterministic for the same input, and a merge puts the second parent in a lane
of its own; the 500 generated histories of the reference test, each within one chunk, lay out as the
reference implementation laid them out; a history of several chunks keeps every parent below its
children, lays out its first chunk exactly as the first chunk alone, never moves a row or lane
already assigned when a chunk is added, and keeps every edge clear of the commits it does not join;
a history whose dates run backwards across a chunk boundary lays its late child out below its
parent, joined by an edge, without moving a row or lane already assigned. The tests of the arena
cite it too: a parent outside the commits laid out is dropped rather than followed, every node
carries its parents and children, and a test counting the layout's allocations holds them to a bound
per chunk, not per commit.
