# ADR-0027: A repository sidebar lists branches, remotes, worktrees, and tags, and selecting a branch or tag reveals its commit

- **Status:** ⚪ superseded by ADR-draft-sidebar-worktrees-filter
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `src-web/src/RepoSidebar.svelte`, `src-web/src/refTree.ts`, `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, and `crates/tui/src/sidebar.rs`

## Context

G-6 shows every local branch, remote branch, and tag as a ref chip on the commit it points at.
That answers *which refs does this commit carry*. It does not answer the inverse, *which refs
exist, and where is one of them*: a branch whose tip lies a thousand rows down is invisible until
the developer scrolls onto it, and search (G-11) answers a lookup by name, not the question of
what there is. Every comparable client answers it with a panel on the left listing local
branches, remote branches, worktrees, and tags.

Forces:

- **The Non-Goal on hosting** rules out the pull-request, issue, and team sections those clients
  put in the same panel; they are excluded, not deferred.
- **No request of Ocelli's own** ([ADR-0025](0025-no-egress-hardened-webview.md)): every number in
  the panel, ahead and behind included, is computed from refs already on disk; nothing fetches.
- **Reading goes through the port** ([ADR-0008](0008-ports-and-adapters.md),
  [ADR-0017](0017-git2-repository-reader.md)): the refs are read by `ocelli-git` through
  `RepositoryReader`, off the thread that draws.
- **The history loads in chunks** ([ADR-0016](0016-history-loads-in-chunks.md)), so a ref can point
  at a commit whose chunk has not loaded yet; the panel must handle that rather than jump nowhere.
- **Synthetic nodes** ([ADR-0018](0018-synthetic-nodes-in-the-first-chunk.md)) are for points in
  history; a worktree is a place a repository is checked out, not one.

`git2` already offers `Repository::worktrees`, `branches`, `tag_names`, `remotes`, and
`graph_ahead_behind`, so no dependency is added.

## Decision

We will add a **repository sidebar** to the left of the commit graph, listing local branches,
remote branches grouped by remote, linked worktrees, and tags, in which selecting a branch or a tag
reveals and selects the commit it points at.

1. **One value, one load.** `RepositoryReader` returns a `RepoRefs` value read in the same pass as
   the history and delivered beside it, in the desktop host as a `load-refs` event right after the
   first chunk. Refs and commit graph always describe the same read of the repository, and
   refreshing one refreshes the other.
2. **`RepoRefs` carries the sidebar's four lists**, serialised in camelCase like every other value
   the web interface receives, beside what other ADRs add to it:
   - `locals`: name, target SHA, whether it is `HEAD`, its configured upstream, and `ahead` and
     `behind` against that upstream when there is one;
   - `remotes`: one entry per configured remote, each with its remote-tracking branches;
   - `worktrees`: the linked worktrees `Repository::worktrees` reports, each with its name, path,
     and checked-out branch and commit;
   - `tags`: name and target SHA, peeled so an annotated tag resolves to its commit.
3. **Selecting navigates.** A branch or a tag calls the same reveal-and-select path as the search
   results and the parent and child links of the commit detail. An entry whose commit lies outside
   the loaded set is drawn inert and says so, rather than silently doing nothing. A worktree
   switches the repository being viewed instead, as
   [ADR-0028](0028-submodules-and-repository-switching.md) decides.
4. **Sections are collapsible and counted**, so that a repository with 300 remote branches does not
   push the tags out of view.

**Out of scope:** the operations on refs, creating, switching, and deleting branches and tags (G-12,
G-16), which [ADR-0031](0031-writes-what-git-writes.md) decides; submodules and what switching the
viewed repository involves, which [ADR-0028](0028-submodules-and-repository-switching.md) decides.

## Alternatives considered

- **The ref tree inside the commit graph's value instead of a second one** — one message less,
  but it welds two concerns: the sidebar would refresh only by rebuilding the whole layout, and the
  graph would carry data its drawing never reads.
- **A call the front end makes after the commit graph arrives** — more flexible in theory, but a
  second path of loading that can disagree with the first, for data that is cheap and always
  wanted. One read keeps "the repository was read" one moment in time.
- **No sidebar, search alone** — search matches ref names, but answers where one is, not what
  there is; the two complement each other.
- **Worktrees as synthetic nodes** — a stash and the working directory are points in history and
  earn a row; a worktree is a checkout location, and putting it in the graph misrepresents both.
- **The sections comparable clients place beside these** (pull requests, issues, teams) —
  excluded by the Non-Goal on hosting.

## Sources / Prior art

- `git2::Repository` and `git2::Worktree`, the API the reader is built on; `worktrees()` reports
  linked worktrees only, so the main working tree is not among them —
  <https://docs.rs/git2/latest/git2/struct.Repository.html>
- `git worktree` — <https://git-scm.com/docs/git-worktree>
- GitKraken's left panel, and the same in Fork and Sourcetree: collapsible sections per kind of
  ref, counts, the checked-out branch marked, ahead and behind as arrow counts.
- GitGraph's `docs/adr/0023-repository-sidebar-ref-tree.md` on `main`, where this sidebar runs.

## Consequences

- Positive: refs become discoverable, not only recognisable; a branch tip far down the history is
  one selection away; nothing is fetched or sent, so
  [ADR-0025](0025-no-egress-hardened-webview.md) holds.
- Negative / trade-offs: a second value and a second event to keep in step with the commit graph;
  a fourth pane competing for width with the commit graph and the commit detail; ahead and behind
  reflect the last fetch and can be stale until the developer fetches.
- Follow-ups: the operations on refs from the sidebar, which the ref chips offer
  ([ADR-0031](0031-writes-what-git-writes.md)); whether the main working tree joins the worktrees
  section; a filter once the sections' sizes justify it; whether stashes earn a section of their
  own beside their nodes; the glossary gains *worktree* and *repository sidebar*.

## Enforcement

The tests of `src-web/src/refTree.test.ts`, `crates/git/src/refs.rs`, and
`crates/tui/src/sidebar.rs` cite this ADR with a `Verifies:` marker: names with slashes group
into folders, remote branches group per remote, a filter opens the folders it matches into, and
selecting an entry yields the ref to reveal.
