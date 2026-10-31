# ADR-0024: Stashes and uncommitted changes appear as synthetic nodes in the first chunk of the commit graph

- **Status:** 🟢 accepted
- **Date:** 2026-10-06
- **Deciders:** Markus Brigl
- **Applies to:** the node kinds and `WORKDIR_SHA` in `crates/core/src/model.rs`, the reading of stashes and the working directory in `crates/git/src/git.rs`, and how the front ends draw them

## Context

The commit graph draws commits, each joined to its parents. Two things a developer looks for in a
history are not commits in that sense:

- **Stash entries** are commits as objects, but they hang off `refs/stash` with a reflog, and each
  has internal parents for the index and the untracked files that are implementation detail.
  Walking them like ordinary refs drags that machinery into the commit graph.
- **Uncommitted changes** are often the most relevant row for someone reading their own
  repository: where am I, and what have I not committed yet. They have no object id at all.

Leaving both out keeps every node a commit, but hides a dirty working directory in the very view
meant to answer "where am I", and stashed work does not exist there. The glossary already names a
**working-directory node** at the top of the commit graph, and G-8 excepts its row from the
determinism of the layout. Ocelli also creates, applies, and drops stashes (G-16), which wants the
stashes to be somewhere the developer can select them.

The layout is a pure function that reads no clock ([ADR-0010](0010-functional-core-layout.md)), so
whatever places the working-directory node at the top has to arrive as a value. The history
loads in chunks of 3 000 commits ([ADR-0016](0016-history-loads-in-chunks.md)), the first of which
is laid out and shown before the rest is read.

## Decision

We will surface stashes and uncommitted changes as **synthetic nodes**: entries that take a row and
a lane in the commit graph like a commit and carry a `kind` that tells them apart from commits.

- **The kind.** Every node carries a `kind` of `commit`, `stash`, or `working`, and the kind is part
  of the contract with every front end.
- **Stash nodes.** Read from the stash reflog, one node per entry, labelled `stash@{n}`.
  `refs/stash` is skipped by the ordinary ref walk, and **only the first parent**, the commit the
  stash was made on, is kept, so the internal index and untracked parents never enter the graph.
- **The working-directory node.** One node, present only when `HEAD` resolves and the working tree
  or the index is dirty, ignored files excluded. It has no object id; the reserved sentinel
  `WORKDIR_SHA` (`WORKING_DIRECTORY`) stands in for one, its parent is `HEAD`, and its timestamp is
  the moment of reading, set by `ocelli-git` when it builds the node, so that the temporal sort of
  [ADR-0015](0015-layout-after-pvigier-by-chunk.md) places it at row 0. The front ends show
  "Uncommitted changes" in place of a SHA.
- **Selecting one yields a diff like any other node**: for the working-directory node, `HEAD`
  against the working tree with the index included and untracked files listed; for a stash node,
  the ordinary diff against its first parent.
- **They look different, not like commits in disguise**: a diamond for a stash, a dashed outline
  for the working-directory node, against a filled circle for a commit (in the terminal `◆`, `◌`,
  and `●`).
- **They ride the first chunk**: the stash and working-directory nodes join the first chunk of the
  load, and count towards its size of `CHUNK_COMMITS`; the newest walked commits that make room
  for them open the second chunk, so no walked commit is lost to them.

**Out of scope:** the operations on stashes and on the working directory (G-12, G-16), which
[ADR-0033](0033-writes-what-git-writes.md) and
[ADR-0034](0034-discarding-work-only-after-confirmation.md) decide; worktrees and submodules, which
are repository state rather than history ([ADR-0026](0026-repository-sidebar.md),
[ADR-0027](0027-submodules-and-repository-switching.md)).

## Alternatives considered

- **Commits only** — the simplest model, every node a commit. Not chosen: it hides the two states a
  developer most often looks for, and Ocelli reads the working directory for diffs anyway.
- **Stashes but no uncommitted changes** — avoids the node without an object id, the one irregular
  case. Not chosen: the dirty working directory is the more useful of the two, and the sentinel
  confines the irregularity to one named constant.
- **Both outside the commit graph, in a list or a status line** — keeps the graph purely made of
  commits, but loses the topology that makes them useful here: uncommitted work sits on top of
  `HEAD`, and a stash hangs off its base commit. A list cannot show that.
- **`refs/stash` walked as an ordinary ref** — no special case at all, but it pulls the index and
  untracked commits into the graph as visible nodes, noise nobody asked for.
- **A timestamp far in the future for the working-directory node** — pins it to row 0
  unconditionally, but is a lie in the data that every comparison by date has to work around; the
  moment of reading achieves the same honestly.
- **Synthetic nodes beyond the first chunk's size** — no walked commit moves, but the first chunk
  grows past the size every other chunk keeps, and its layout no longer equals a chunk of
  `CHUNK_COMMITS` commits alone.

## Sources / Prior art

- `git stash` and the structure of stash commits — <https://git-scm.com/docs/git-stash>
- `git status`, what counts as dirty — <https://git-scm.com/docs/git-status>
- GitKraken and `gitk`, which both show a row for uncommitted changes at the top of the graph —
  <https://www.gitkraken.com/>
- libgit2, `git_stash_foreach` and `git_diff_tree_to_workdir_with_index` — <https://libgit2.org/>
- GitGraph's `docs/adr/0021-synthetic-stash-and-workdir-nodes.md` on `main`, where these nodes run.

## Consequences

- Positive: the two states a developer looks for are where they look, in topological relation to
  the commits they belong to; the diff path is uniform, so selecting any node behaves the same;
  the kind keeps them honest; stash internals stay out of the graph; the stash operations of G-16
  have a node to act on.
- Negative / trade-offs: a node is no longer always a commit, so every consumer handles `kind`, and
  code that expects an object id special-cases the sentinel; the working-directory node's timestamp
  comes from the clock, so its row is the one part of the layout that is not a function of
  repository state (G-8 records the exception); a repository gains or loses that row without any
  commit changing.
- Follow-ups: the glossary gains *synthetic node* and *stash node*.

## Enforcement

The tests of the git adapter cite this ADR with a `Verifies:` marker for the first chunk moving
walked commits on to make room for the working-directory node, and for the load reference test's
synthetic nodes; the tests of `crates/core` for the working-directory node carrying no short SHA
and its links labelled by name; the tests of `crates/tui/src/raster.rs` for each synthetic kind
getting a glyph of its own rather than passing for a commit; and the measurement's test for both
being left out of the comparison with git.
