# ADR-0016: The history loads in chunks of 3 000 commits, the newest first, until all of it is loaded

- **Status:** 🟢 accepted
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the revwalk in `crates/git/src/git.rs`, the repository port in `crates/core/src/port.rs`, the chunk size in `crates/core/src/model.rs`, and every front end's load

## Context

Q-2 asks that a repository of a million commits stays usable: the first screen within 2 s of
opening it, and no wait above 100 ms while scrolling. Loading reads each commit's metadata on a
walk over every ref; on a million commits that walk alone takes seconds, so a load that finishes
before anything is shown cannot meet Q-2. Cutting the history at a fixed bound keeps the worst
case predictable, but leaves most of a large history unseen.

The maintainer decided that the newest commits appear at once and older ones load in the
background while the developer already scrolls, and that the first screen holds the newest 3 000
commits, a bound that keeps the first load and layout predictable whatever the repository's size.

The order of the walk decides when the first commit arrives. A topologically sorted walk, which
never yields a commit before one of its children, first reads the whole history: on a generated
repository of a million commits libgit2 takes 3.0 s before the first commit, and `git log
--topo-order` 2.4 s, without a commit-graph file to read generation numbers from. The walk in
commit-date order, git's default, yields the first commit at once and the first 3 000 within
20 ms; it yields a commit before one of its children only when that child carries an older
committer date than the commit, where a clock ran wrong.

## Decision

We will load the history in **chunks of 3 000 commits** in the **commit-date order** of the walk
over every ref, without a topological sort: the walk starts from the refs' commits, newest first,
keeps the commits it has reached ordered by committer date, and yields each once one of its
children has been yielded. The first chunk is laid out and shown as soon as it is read, the stash
and working-directory nodes ride with it
([ADR-0024](0024-synthetic-nodes-in-the-first-chunk.md)), and every further chunk is read on a
background thread and appended until the walk ends, with the progress reported (G-13) and a load
of another repository cancelling the walk of the previous one (a new load session). A commit
yielded after its parent, a **late child**, is laid out as
[ADR-0015](0015-layout-after-pvigier-by-chunk.md) decides.

**Out of scope:** how a chunk is laid out, which [ADR-0015](0015-layout-after-pvigier-by-chunk.md)
decides; how a front end reads the rows, which [ADR-0031](0031-front-ends-read-row-windows.md)
decides; loading only what is scrolled to (the maintainer chose to load the whole history); a
chunk size the developer sets.

## Alternatives considered

- **A fixed bound on the load** — predictable, but most of a large history stays unseen and
  Q-2 cannot be met.
- **Load everything, then show** — no new mechanism, but on a million commits the first screen
  waits for the whole walk.
- **Load on demand while scrolling** — the least memory, but search and jumping to the end need
  history that is not loaded yet; the maintainer chose the whole history in the background.
- **A smaller first chunk** — an earlier first screen, but fewer commits on it, while the first
  3 000 already arrive within 20 ms.
- **Chunks by time window (the last twelve months)** — bounds nothing: an active repository
  produces more commits in a year than a dormant one in a decade.
- **A topologically sorted walk** — no commit before its children, ever, but the first commit
  waits for the whole history to be read: 3.3 s to the first window at a million commits, against
  Q-2's 2 s.
- **A topological walk on generation numbers** — git's incremental `--topo-order`, but it needs a
  commit-graph file, which a repository may lack and libgit2's walk does not read, and a reader of
  Ocelli's own besides libgit2.

## Sources / Prior art

- libgit2's revwalk, which hands commits over one by one: unsorted, in commit-date order as
  described above; with any sorting requested, only once it has read the whole history —
  <https://libgit2.org/libgit2/#HEAD/group/revwalk>
- `git log`'s default order and `--topo-order`, with the generation numbers of the commit-graph
  file that make the latter incremental — <https://git-scm.com/docs/git-log>,
  <https://git-scm.com/docs/commit-graph>
- GitKraken and `gitk`, which load history in windows rather than all at once —
  <https://www.gitkraken.com/>
- GitGraph's `docs/adr/0011-commit-limit-3000.md` on `main`, the bound of 3 000 commits that
  sets the size of a chunk.
- The Q-2 measurement of feature spec #27 (`scripts/measure-windows.sh`), which gave the times
  above.

## Consequences

- Positive: the first screen does not wait for the whole history (Q-2); the whole history becomes
  reachable; where no committer date runs backwards and no two commits at the end of the first
  chunk share a date, the first chunk holds the same commits a topological walk would, so the
  reference tests hold.
- Negative / trade-offs: where a clock ran wrong, a late child arrives after its parent and cannot
  be placed above it (ADR-0015); a load runs on until the whole history is read, and memory grows
  with it; the stash and working-directory nodes are placed against the first chunk only;
  cancelling a walk is state in every front end; the first chunk of a corpus repository whose
  dates run backwards may hold other commits than a topological walk would, which the Q-1 records
  reflect.
- Follow-ups: the Q-1 records measured on the corpus over the first chunk.

## Enforcement

A test of the git adapter loads a generated repository of several chunks and checks that the
chunks come in commit-date order, that the first chunk holds the newest commits, and that together
they hold the whole history, and one whose dates run backwards checks that every commit still
arrives once; they cite this ADR with a `Verifies:` marker. The measurement of Q-2 cites it too.
