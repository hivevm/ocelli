# Ticket: Both merge editors decide alike what is still a block once the developer edits

<!-- Filed by the design revision of 2026-11-03. Decided in docs/adr/0004-feature-layer.md. -->

## What to build

The core finds a conflicted file's blocks once, when the file is read (ADR-0031). From then on each
front end decides on its own what is still a block, and they decide differently:

- the web interface (`src-web/src/mergeBlocks.ts`) keeps the blocks it opened with, their marker
  lines mapped through each edit by CodeMirror, and a block whose markers an edit moved out of order
  or removed draws nothing; a run of marker lines the developer types or pastes is never a block;
- the terminal application (`crates/tui/src/merge.rs`) asks the core's `blocks` for the blocks of
  the text as it is now on every key, so a typed or pasted run of markers is a block with its
  actions, and finds a block's base again by matching its ours and theirs
  (`MergeEditor::base_here`).

Around that, what a line of a block is (`lineKinds` / `line_kinds`), what taking both sides puts in
a block's place (`sideText` / `take`), and whether a result is unsaved (`unsaved`) are each written
twice. docs/CONVENTIONS.md, "A behaviour both front ends offer lives once in the core; only drawing
is ported", names the smell: two answers to one question drift into two products. The web interface
cannot ask the core on every key, so the question for the human is which rule both follow, and
whether the rest moves into the core as values the host sends or stays ported with a shared fixture
both suites read.

## Acceptance criteria

- [ ] Given a decision on what is a block after an edit, when it is written down (ADR-0031's
      restatement, or a `proposed` ADR if it is a new decision), then both front ends follow it.
- [ ] Given a file where the developer pastes a whole run of marker lines, when each front end draws
      it, then both treat it alike, and a test in each suite says so.
- [ ] Given the same fixture of blocks, when each front end takes ours, theirs, or both, then the
      results are the same text, read by both suites from one fixture as
      `crates/core/tests/fixtures/` already serves the web interface's types.

## Read first

- docs/adr/0031-writes-what-git-writes.md, section *The merge editor*;
  docs/adr/0022-desktop-host-answers-what-the-core-decides.md; docs/adr/0008-ports-and-adapters.md
- docs/CONVENTIONS.md, "A behaviour both front ends offer lives once in the core" and "Logic lives
  in a pure helper beside the component"
- `crates/core/src/merge.rs`, `crates/tui/src/merge.rs`, `src-web/src/mergeBlocks.ts`,
  `src-web/src/mergeEditor.ts`, `crates/core/tests/write_fixture.rs`

## Blocked by

- None (can start immediately)

## Mode

with the human — which rule both editors follow is a behaviour of G-15 the human decides; the case
is rare (pasted markers), so the human may also decide to leave the two as they are and say so in
ADR-0031's restatement.
