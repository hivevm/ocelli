# Ticket: Draft — Q-7 names leaving the merge editor with a result not saved

<!-- Filed by the design revision of 2026-11-03. A drafted specification change: only a human
     lands it (AGENTS.md, section 3, rule 7). Decided in docs/adr/0004-feature-layer.md. -->

## What to build

Q-7 of docs/SPECIFICATION.md is measured "for each such operation (a hard reset, dropping a stash,
applying a stash and dropping it, deleting a tag, deleting a branch that is not merged, aborting an
operation in progress, taking one side of a conflicted file)". The merge editor's tests cite Q-7 for
a case the list does not name: leaving the editor with a result not saved
(`src-web/src/mergeEditor.test.ts`, `crates/tui/src/merge.rs`). ADR-0031 decides that leaving asks
first "(Q-7)", and the closed spec served Q-7 by it. The text no longer says everything the tests
decide.

Drafted change, for the human to land or reject:

> Measured for each such operation (a hard reset, dropping a stash, applying a stash and dropping
> it, deleting a tag, deleting a branch that is not merged, aborting an operation in progress,
> taking one side of a conflicted file, leaving the merge editor with a result not saved): it runs
> only after the developer confirms it.

Accepted ADRs whose basis it moves: ADR-0033 (its *Context* lists Q-7's operations and its core list
is what the front ends ask for; leaving the editor is asked by the front end, outside that list, as
ADR-0031 states) and ADR-0031 (*Leaving*). Neither changes in substance; the human judges that, not
the agent.

A second question for the human, no draft: Q-3 measures the desktop application's memory on the
commit graph alone. Ticket merge-editor/06 measured the WebView adding 180 / 210 MB on files of
70,004 and 35,009 lines open in the merge editor, more than Q-3's 150 MB, on a case Q-3 does not
measure; the maintainer decided on no limit. Whether Q-3 should say so is open.

## Acceptance criteria

- [ ] Given the draft, when the human decides, then either Q-7 names leaving the merge editor and
      the commit names ADR-0033 and ADR-0031, or the tests' `Verifies: Q-7` stays as it is and the
      human says why.
- [ ] Given the Q-3 question, when the human answers, then the answer is in the specification or in
      the hand-off.

## Read first

- docs/SPECIFICATION.md, Q-3 and Q-7
- docs/adr/0033-discarding-work-only-after-confirmation.md; docs/adr/0031-writes-what-git-writes.md,
  *Leaving*
- `git show 970f6cf` for the measurements

## Blocked by

- None (can start immediately)

## Mode

with the human — a change to the specification is the human's
([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 7).
