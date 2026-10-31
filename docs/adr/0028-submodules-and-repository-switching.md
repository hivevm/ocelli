# ADR-0028: Submodules appear in the repository sidebar, and the sidebar switches the viewed repository

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `crates/git/src/refs.rs`, the `RepoRefs` value in `crates/core/src/model.rs`, the sidebar's submodule, worktree, and parent rows, and the flow that switches the viewed repository

## Context

The repository sidebar of [ADR-0027](0027-repository-sidebar.md) answers what a repository
consists of: its branches, remote branches, worktrees, and tags. A submodule belongs to the same
answer. It is repository state in the sense a worktree is, something the repository contains and
points at, and invisible in a view that only draws history.

One asymmetry shapes this decision. A worktree's `HEAD` is a commit in this repository's object
database. A submodule's recorded id is a commit in the submodule's object database: it is never a
row in this commit graph, and never in the set of loaded commits the sidebar tests against.
Revealing it is impossible, and an inert row would explain itself with the wrong reason ("outside
the loaded history" rather than "in another repository"). What a submodule row does instead is
open the submodule: its working directory is a repository on disk, and opening it is a read. The
same holds for a worktree, whose row switches to it. Moving into a submodule or across worktrees
must be reversible, so "which repository is being viewed" becomes something the developer moves
through, not only a choice in the folder picker.

Forces:

- **Showing starts nothing and fetches nothing.** Reading `.gitmodules`, the index entry, a
  submodule's checked-out commit, and another worktree's `HEAD` is reading. An uninitialised
  submodule is shown as uninitialised: cloning it would be a request
  [ADR-0025](0025-no-egress-hardened-webview.md) does not allow while showing, and opening a
  submodule is held to Q-4 like any repository.
- **The chunked load** ([ADR-0016](0016-history-loads-in-chunks.md)) makes an ordinary ref inert,
  while its chunk has not loaded, for a different reason than a submodule's; the two must not be
  confused on screen.
- **Reading goes through the port** in `ocelli-git` ([ADR-0017](0017-git2-repository-reader.md)).
- **Repository state is not a graph row**, the precedent of
  [ADR-0018](0018-synthetic-nodes-in-the-first-chunk.md) and
  [ADR-0027](0027-repository-sidebar.md).

`git2` already offers `Repository::submodules`, `Submodule::{name, path, url, head_id,
workdir_id}`, and, for the way back, `Repository::{is_worktree, commondir, discover}`, so no
dependency is added. Opening a repository already runs the whole load, so switching is a call the
sidebar can make.

## Decision

We will add a fifth section, **Submodules**, to the repository sidebar and let the sidebar
**switch the repository being viewed**: into a submodule, between worktrees, and back up to the
parent, through the ordinary load of a repository.

1. **`RepoRefs` gains a fifth list**, `submodules`, read in the same pass and delivered with the
   same value as the other four, so refs, submodules, and commit graph still describe one read.
2. **Each submodule entry carries** its name and path, the working copy's absolute path, its
   configured URL, the commit the superproject records (`head_id`), the commit checked out
   (`workdir_id`), and a state derived from the two: *uninitialised* (no working copy on disk),
   *modified* (checked out at another commit than recorded), or in step.
3. **Selecting a submodule opens it** as the repository being viewed. An uninitialised submodule
   has no working directory, so its row is inert and says why.
4. **Selecting a worktree switches to it.** Linked worktrees share the object database, so the
   history is the same; what changes is `HEAD`, the branch marked as checked out, and the
   working-directory node. The worktree being viewed is marked and inert, like a branch that is
   already `HEAD`.
5. **`RepoRefs` names the parent**, so the way back is a fact read from Git rather than a trail the
   front end remembers: for a linked worktree, the main working tree found through `commondir`;
   for a submodule, the superproject found by discovering upwards from the working copy and
   confirming that one of its submodules resolves to this path. The confirmation keeps an unrelated
   repository that merely sits above this one from being presented as its parent. The sidebar shows
   the parent as a row above the sections, absent when there is none.
6. **The section behaves like the other four**: collapsible, counted, filtered by the shared filter
   field, and present in the collapsed rail with a glyph of its own. Submodules are listed one level
   deep, as the superproject configures them; opening one shows its own.

**Out of scope:** initialising, updating, syncing, or removing submodules and adding or pruning
worktrees, which belong with the decisions on Ocelli's writes, none of which, in
[ADR-0031](0031-writes-what-git-writes.md), offers them; opening a submodule at its recorded commit
rather than at its `HEAD`; a list of recent repositories or a back stack of several steps.

## Alternatives considered

- **Inert submodule rows, for information only** — the smallest change, but a whole section of
  dead rows, although the submodule is a repository in a known local directory; it remains only for
  the uninitialised case.
- **A navigation stack in the front end** — remember where the developer came from and offer
  "back". Simple and never wrong, but it helps only after a selection: a submodule opened from the
  folder picker would have no way up, although Git knows its superproject. A stack can still be
  added on top for several steps.
- **Revealing the recorded commit in the current commit graph** — impossible: the id belongs to the
  submodule's object database, and the jump would silently miss.
- **The submodule's history drawn inside the superproject's graph** — two object databases in one
  graph misrepresent both and invent ancestry that does not exist.
- **Synthetic nodes for submodules** — a submodule is a location and a recorded commit, not a point
  in this repository's history.
- **Worktree rows that reveal the commit, with a separate action to switch** — two actions per row,
  for a commit nearly always reachable through its branch in the local section anyway.
- **Reading `.gitmodules` directly** — the file names the configured submodules but not what is
  checked out, and disagrees with the index for one configured but not initialised.
- **A separate call or event for submodules** — a second path of loading that can disagree with
  the first, for data that is cheap and always wanted.

## Sources / Prior art

- `git2::Repository::submodules` and `git2::Submodule`: `head_id` is the id recorded in the
  superproject's `HEAD` tree, `workdir_id` the one checked out —
  <https://docs.rs/git2/latest/git2/struct.Submodule.html>
- `git2::Repository::commondir`, the shared Git directory that leads from a linked worktree to the
  main one — <https://docs.rs/git2/latest/git2/struct.Repository.html>
- `git rev-parse --show-superproject-working-tree`, Git's own answer to which repository contains
  this one, which clause 5 reproduces through `git2` — <https://git-scm.com/docs/git-rev-parse>
- `git submodule` and `gitmodules` — <https://git-scm.com/docs/git-submodule>,
  <https://git-scm.com/docs/gitmodules>
- GitKraken, Fork, and Sourcetree, which list submodules in a section beside worktrees and open a
  submodule as its own repository.
- GitGraph's `docs/adr/0024-submodules-and-repository-switching.md` on `main`, where this
  switching runs.

## Consequences

- Positive: a submodule stops being invisible; the panel shows that it exists, which commit it is
  recorded at, and whether its working copy has drifted, all read from disk without a fetch; moving
  into a submodule, across worktrees, and back up is navigation rather than a trip through the
  folder picker, and the parent row makes it reversible from wherever the developer lands.
- Negative / trade-offs: the sidebar's action on selection is not uniform, branches and tags reveal
  a commit while submodules and worktrees switch the repository, which the rows must convey by
  glyph, tooltip, and a visible reload; switching drops the selection and the scroll position,
  since it is a full load; submodule state is a snapshot that goes stale like ahead and behind; the
  parent lookup opens one repository above the current one on every load.
- Follow-ups: the operations on submodules and worktrees; a list of recent repositories or a back
  stack; opening a submodule at its recorded commit; marking the commits that change a submodule's
  recorded commit; the glossary gains *submodule* and *superproject*.

## Enforcement

The tests of `crates/git/tests/repo_state.rs` and `crates/tui/src/sidebar.rs` cite this ADR with a
`Verifies:` marker: a submodule is reported as modified or uninitialised without being fetched, the
parent repository is read from Git rather than remembered, and the worktree being viewed is inert.
`src-web/src/refTree.test.ts` cites it for the fifth section grouping and filtering like the other
four.
