# ADR-0035: A repository sidebar lists branches, remotes, every worktree, tags, and submodules under one filter, selecting a branch or tag reveals its commit, and selecting a worktree or submodule switches the viewed repository

- **Status:** 🟢 accepted
- **Date:** 2026-10-31
- **Deciders:** Markus Brigl
- **Applies to:** `src-web/src/RepoSidebar.svelte`, `src-web/src/refTree.ts`, `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, `crates/tui/src/sidebar.rs`, the sidebar's submodule, worktree, and parent rows, and the flow that switches the viewed repository

## Context

G-6 shows every local branch, remote branch, and tag as a ref chip on the commit it points at.
That answers *which refs does this commit carry*. It does not answer the inverse, *what does this
repository consist of, and where is one of its parts*: a branch whose tip lies a thousand rows down
is invisible until the developer scrolls onto it, search (G-11) answers a lookup by name, not the
question of what there is, and a submodule or another worktree is invisible in a view that only
draws history. Every comparable client answers it with a panel on the left listing local branches,
remote branches, worktrees, tags, and submodules.

One asymmetry shapes what selecting an entry does. A branch or a tag points at a commit in this
repository's object database, and a worktree's `HEAD` does too. A submodule's recorded id is a
commit in the submodule's object database: it is never a row in this commit graph, and never in
the set of loaded commits the sidebar tests against. Revealing it is impossible, and an inert row
would explain itself with the wrong reason ("outside the loaded history" rather than "in another
repository"). A submodule's working directory, though, is a repository on disk, and opening it is
a read; the same holds for another worktree. Moving into a submodule or across worktrees must be
reversible, so "which repository is being viewed" becomes something the developer moves through,
not only a choice in the folder picker.

Forces:

- **The Non-Goal on hosting** rules out the pull-request, issue, and team sections comparable
  clients put in the same panel; they are excluded, not deferred.
- **Showing starts nothing and fetches nothing** ([ADR-0033](0033-no-egress-hardened-webview.md)):
  every number in the panel, ahead and behind included, is computed from refs already on disk.
  Reading `.gitmodules`, the index entry, a submodule's checked-out commit, and another worktree's
  `HEAD` is reading. An uninitialised submodule is shown as uninitialised: cloning it would be a
  request that decision does not allow while showing, and opening a submodule is held to Q-4 like
  any repository.
- **Reading goes through the port** ([ADR-0009](0009-ports-and-adapters.md),
  [ADR-0021](0021-git2-repository-reader.md)): the refs and submodules are read by `ocelli-git`
  through `RepositoryReader`, off the thread that draws.
- **The history loads in chunks** ([ADR-0018](0018-history-loads-in-chunks.md)), so a ref can point
  at a commit whose chunk has not loaded yet; the panel must handle that rather than jump nowhere,
  and must not confuse it on screen with a submodule's commit, which no chunk will ever bring.
- **Repository state is not a graph row.** Synthetic nodes
  ([ADR-0019](0019-synthetic-nodes-in-the-first-chunk.md)) are for points in history; a worktree
  is a place a repository is checked out, and a submodule a location and a recorded commit.

`git2` already offers `Repository::worktrees`, `branches`, `tag_names`, `remotes`,
`graph_ahead_behind`, `submodules`, `Submodule::{name, path, url, head_id, workdir_id}`, and, for
the way back, `Repository::{is_worktree, commondir, discover}`, so no dependency is added.
Opening a repository already runs the whole load, so switching is a call the sidebar can make.

## Decision

We will add a **repository sidebar** to the left of the commit graph, listing local branches,
remote branches grouped by remote, the main working tree and the linked worktrees, tags, and
submodules, narrowed by one filter, in which selecting a branch or a tag reveals and selects the
commit it points at, and selecting a worktree or a submodule **switches the repository being
viewed**, with a way back up to the parent, through the ordinary load of a repository.

1. **One value, one load.** `RepositoryReader` returns a `RepoRefs` value read in the same pass as
   the history and delivered beside it, in the desktop host as a `load-refs` event right after the
   first chunk. Refs, submodules, and commit graph always describe the same read of the
   repository, and refreshing one refreshes the other.
2. **`RepoRefs` carries the sidebar's five lists**, serialised in camelCase like every other value
   the web interface receives, beside what other ADRs add to it:
   - `locals`: name, target SHA, whether it is `HEAD`, its configured upstream, and `ahead` and
     `behind` against that upstream when there is one;
   - `remotes`: one entry per configured remote, each with its remote-tracking branches;
   - `worktrees`: the main working tree first, read through the shared git directory so that it
     appears once when a linked worktree is viewed, then the linked worktrees
     `Repository::worktrees` reports, each with its name, path, and checked-out branch and commit,
     the main one and the one being viewed marked as such; a bare repository has no main one;
   - `tags`: name and target SHA, peeled so an annotated tag resolves to its commit;
   - `submodules`: each with its name and path, the working copy's absolute path, its configured
     URL, the commit the superproject records (`head_id`), the commit checked out (`workdir_id`),
     and a state derived from the two: *uninitialised* (no working copy on disk), *modified*
     (checked out at another commit than recorded), or in step.
3. **`RepoRefs` names the parent**, so the way back is a fact read from Git rather than a trail the
   front end remembers: for a linked worktree, the main working tree found through `commondir`;
   for a submodule, the superproject found by discovering upwards from the working copy and
   confirming that one of its submodules resolves to this path. The confirmation keeps an unrelated
   repository that merely sits above this one from being presented as its parent. The sidebar shows
   the parent as a row above the sections, absent when there is none.
4. **Selecting a branch or a tag navigates.** It calls the same reveal-and-select path as the
   search results and the parent and child links of the commit detail. An entry whose commit lies
   outside the loaded set is drawn inert and says so, rather than silently doing nothing.
5. **Selecting a worktree switches to it.** Linked worktrees share the object database, so the
   history is the same; what changes is `HEAD`, the branch marked as checked out, and the
   working-directory node. The worktree being viewed is marked and inert, like a branch that is
   already `HEAD`.
6. **Selecting a submodule opens it** as the repository being viewed. An uninitialised submodule
   has no working directory, so its row is inert and says why. Submodules are listed one level
   deep, as the superproject configures them; opening one shows its own.
7. **Sections are collapsible and counted**, so that a repository with 300 remote branches does not
   push the tags out of view. The Submodules section is present in the collapsed rail with a glyph
   of its own, as the other four are.
8. **One filter narrows every section at once**, in both front ends: an entry stays where its full
   name, slashes included, contains the filter's text, ignoring case and the text's surrounding
   blanks; a folder holding a match opens, and a section counts what passes.

**Out of scope:** the operations on refs, creating, switching, and deleting branches and tags (G-12,
G-16), which [ADR-0022](0022-writes-what-git-writes.md) decides; initialising, updating, syncing,
or removing submodules and adding or pruning worktrees, which belong with the decisions on Ocelli's
writes, none of which, in [ADR-0022](0022-writes-what-git-writes.md), offers them; opening a
submodule at its recorded commit rather than at its `HEAD`; a list of recent repositories or a back
stack of several steps.

## Alternatives considered

- **The ref tree inside the commit graph's value instead of a second one** — one message less,
  but it welds two concerns: the sidebar would refresh only by rebuilding the whole layout, and the
  graph would carry data its drawing never reads.
- **A call the front end makes after the commit graph arrives, or a separate call or event for
  submodules** — more flexible in theory, but a second path of loading that can disagree with the
  first, for data that is cheap and always wanted. One read keeps "the repository was read" one
  moment in time.
- **No sidebar, search alone** — search matches ref names, but answers where one is, not what
  there is; the two complement each other.
- **The linked worktrees alone** — what `Repository::worktrees` reports, and what the sidebar
  first listed, with no filter and the main working tree left as a follow-up; but the checkout
  the developer most often returns to is then the one the sidebar cannot reach, so it did not
  stand, and the filter came with the main working tree.
- **No filter, or one per section** — no field to keep, or a narrower match, but a long list of
  remote branches is scrolled by hand, and one field over every section finds a name wherever it
  is filed.
- **Prefix or fuzzy matching** — fewer or more matches than a plain substring, but a name is found
  by any part of it, folders included, the way the search of the commit graph finds text.
- **Worktrees or submodules as synthetic nodes** — a stash and the working directory are points in
  history and earn a row; a worktree is a checkout location and a submodule a location and a
  recorded commit, and putting either in the graph misrepresents both.
- **Inert submodule rows, for information only** — the smallest change, but a whole section of
  dead rows, although the submodule is a repository in a known local directory; it remains only for
  the uninitialised case.
- **A navigation stack in the front end** — remember where the developer came from and offer
  "back". Simple and never wrong, but it helps only after a selection: a submodule opened from the
  folder picker would have no way up, although Git knows its superproject. A stack can still be
  added on top for several steps.
- **Revealing a submodule's recorded commit in the current commit graph** — impossible: the id
  belongs to the submodule's object database, and the jump would silently miss.
- **The submodule's history drawn inside the superproject's graph** — two object databases in one
  graph misrepresent both and invent ancestry that does not exist.
- **Worktree rows that reveal the commit, with a separate action to switch** — two actions per row,
  for a commit nearly always reachable through its branch in the local section anyway.
- **Reading `.gitmodules` directly** — the file names the configured submodules but not what is
  checked out, and disagrees with the index for one configured but not initialised.
- **The sections comparable clients place beside these** (pull requests, issues, teams) —
  excluded by the Non-Goal on hosting.

## Sources / Prior art

- `git2::Repository` and `git2::Worktree`, the API the reader is built on; `worktrees()` reports
  linked worktrees only, so the main working tree is not among them; `commondir` is the shared
  Git directory that leads from a linked worktree to the main one —
  <https://docs.rs/git2/latest/git2/struct.Repository.html>
- `git2::Repository::submodules` and `git2::Submodule`: `head_id` is the id recorded in the
  superproject's `HEAD` tree, `workdir_id` the one checked out —
  <https://docs.rs/git2/latest/git2/struct.Submodule.html>
- `git rev-parse --show-superproject-working-tree`, Git's own answer to which repository contains
  this one, which clause 3 reproduces through `git2` — <https://git-scm.com/docs/git-rev-parse>
- `git worktree` — <https://git-scm.com/docs/git-worktree>; `git submodule` and `gitmodules` —
  <https://git-scm.com/docs/git-submodule>, <https://git-scm.com/docs/gitmodules>
- GitKraken's left panel, and the same in Fork and Sourcetree: collapsible sections per kind of
  ref, counts, the checked-out branch marked, ahead and behind as arrow counts, submodules in a
  section beside worktrees, and a submodule opened as its own repository.
- GitGraph's `docs/adr/0023-repository-sidebar-ref-tree.md` and
  `docs/adr/0024-submodules-and-repository-switching.md` on `main`, where this sidebar and this
  switching run.

## Consequences

- Positive: refs become discoverable, not only recognisable; a branch tip far down the history is
  one selection away; every checkout of the repository, the main one included, is one selection
  away; a ref is found by any part of its name in every section at once; a submodule stops being
  invisible, the panel showing that it exists, which commit it is recorded at, and whether its
  working copy has drifted; moving into a submodule, across worktrees, and back up is navigation
  rather than a trip through the folder picker, and the parent row makes it reversible from
  wherever the developer lands; nothing is fetched or sent, so
  [ADR-0033](0033-no-egress-hardened-webview.md) holds.
- Negative / trade-offs: a second value and a second event to keep in step with the commit graph;
  a fourth pane competing for width with the commit graph and the commit detail; ahead and behind
  reflect the last fetch and can be stale until the developer fetches, and submodule state is a
  snapshot that goes stale the same way; the sidebar's action on selection is not uniform,
  branches and tags reveal a commit while submodules and worktrees switch the repository, which the
  rows must convey by glyph, tooltip, and a visible reload; switching drops the selection and the
  scroll position, since it is a full load; the parent lookup opens one repository above the
  current one on every load.
- Follow-ups: the operations on refs from the sidebar, which the ref chips offer
  ([ADR-0022](0022-writes-what-git-writes.md)); the operations on submodules and worktrees; whether
  stashes earn a section of their own beside their nodes; a list of recent repositories or a back
  stack; opening a submodule at its recorded commit; marking the commits that change a submodule's
  recorded commit; the glossary gains *worktree*, *repository sidebar*, *submodule*, and
  *superproject*.

## Enforcement

The tests of `src-web/src/refTree.test.ts`, `crates/git/src/refs.rs`, and
`crates/tui/src/sidebar.rs` cite this ADR with a `Verifies:` marker: names with slashes group
into folders, remote branches group per remote, a filter opens the folders it matches into, and
selecting an entry yields the ref to reveal. The filter's tests cite it for matching the full
name ignoring case and for every section narrowed at once; a test of `crates/git/src/refs.rs`
cites it for the main working tree, listed first and once, from the main working tree and from a
linked one, and for a bare repository listing none. `src-web/src/refTree.test.ts` cites it for the
Submodules section grouping and filtering like the other four.

The tests of `crates/git/tests/repo_state.rs` and `crates/tui/src/sidebar.rs` cite this ADR with a
`Verifies:` marker: a submodule is reported as modified or uninitialised without being fetched, the
parent repository is read from Git rather than remembered, and the worktree being viewed is inert.
