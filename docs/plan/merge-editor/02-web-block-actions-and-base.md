# Ticket: Take a side per block and open its base in the web interface's merge editor

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

In the merge editor of the web interface, each block is set apart in the theme's tokens, ours
above theirs, with actions that replace the block by ours, by theirs, or by both, ours first. A
block's base is folded beneath it and opens on request; where the file holds no base section, the
core takes the base from a merge of the index's three stages in memory in the `diff3` style where
its block holds the same ours and theirs, and a block without either shows none.

## Acceptance criteria

- [ ] Given a file without base sections whose index holds a base, when the core reads it, then
      each block whose ours and theirs match a block of the in-memory `diff3` merge carries that
      block's base, and every other block none.
- [ ] Given a block, when the developer takes ours, theirs, or both, then the block's lines are
      replaced by that side, or by ours followed by theirs, and undo puts the block back.
- [ ] Given a block with a base, when the developer opens it, then the base shows beneath the
      block, and closing it folds it again; a block without a base offers none.
- [ ] Given the dark and the light theme, when the editor draws, then ours, theirs, and the base are
      set apart by the theme's tokens alone.

## Read first

- docs/adr/0031-writes-what-git-writes.md (The merge editor).
- docs/adr/0024-themes-tokens-and-palettes.md for the tokens.
- docs/adr/0023-svelte-web-interface.md.
- Golden path: ticket 01's editor and its pure-function tests.
- Seams: the core's tests of the blocks; the web interface's pure-function tests of the actions.

## Blocked by

- docs/plan/merge-editor/01-web-merge-editor-tracer.md

## Mode

with the human: the harness ADR keeps the cap on unattended work at zero, and the look of the
blocks in both themes is checked by hand.
