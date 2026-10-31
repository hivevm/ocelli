# Ticket: Resolve a conflicted file in the terminal application's merge editor

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

In the terminal application's conflict dialog, `m` opens the merge editor full screen on a file the
core says the editor opens, on `tui-textarea-2` over ratatui 0.30, from the same blocks the core
finds. Alt-o, Alt-t, and Alt-b take ours, theirs, or both for the block under the cursor; Alt-n and
Alt-p move to the next and the previous block; Alt-a opens and closes the block's base; Ctrl-S
saves, resolving the file and closing the editor; Esc leaves, asking first where the result is not
saved. It fits an 80-column terminal in one column.

## Acceptance criteria

- [ ] Given the conflict dialog on a file the editor opens, when the developer presses `m`, then
      the editor opens on the file's text with its blocks set apart; on any other file `m` does
      nothing and says why.
- [ ] Given the cursor in a block, when Alt-o, Alt-t, or Alt-b is pressed, then the block becomes
      ours, theirs, or both, as in the web interface; Alt-n and Alt-p move between blocks; Alt-a
      opens and closes the base.
- [ ] Given a result without marker lines, when Ctrl-S is pressed, then the file is resolved as in
      ticket 01 and the dialog shows the remaining conflicts; a refused save says why and keeps
      the editor open.
- [ ] Given a result not saved, when Esc is pressed, then the application asks first, and leaving
      without confirming keeps the editor open.
- [ ] `tui-textarea-2` enters as the terminal application's one new dependency, pinned in the
      lockfile, with no second ratatui in the tree.

## Read first

- docs/adr/0031-writes-what-git-writes.md (The merge editor).
- docs/adr/0029-terminal-application-ratatui.md and docs/adr/0024-themes-tokens-and-palettes.md.
- docs/adr/0033-discarding-work-only-after-confirmation.md.
- Golden path: the conflict dialog and its key handling, and the tag and stash prompts.
- Seams: the terminal application's key-handling tests on a fixture of blocks.

## Blocked by

- docs/plan/merge-editor/02-web-block-actions-and-base.md

## Mode

with the human: the harness ADR keeps the cap on unattended work at zero, and the editor is
checked by hand in a real terminal.
