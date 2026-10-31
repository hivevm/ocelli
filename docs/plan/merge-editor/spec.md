# Spec: A merge editor in every front end

<!-- Copy to docs/plan/<spec>/spec.md. The destination of a piece of work larger than one agent
     session, written from the user's side; its tickets lie next to it in the same directory. It
     is disposable: the design revision deletes it once its tickets have merged, and whatever has
     to last moves into docs/SPECIFICATION.md, an ADR, docs/ARCHITECTURE.md, docs/GLOSSARY.md, or
     docs/CONVENTIONS.md first. No file paths or code; they go stale. How it is written is the
     plan-feature skill under .agents/skills/. Decided in docs/adr/0004-feature-layer.md. -->

## Problem

A merge, rebase, cherry-pick, or revert stopped on a conflict leaves files with conflict markers.
Today the developer can take ours or theirs for a whole file, or edit the file in an editor of
their own and come back to mark it resolved. A file with several blocks, where some want ours and
some theirs, or a block that wants a hand-written mix, sends the developer out of Ocelli.

## Solution

Next to each conflicted file whose sides are both text, the developer opens a merge editor. It
shows the file as it lies in the working directory, each conflict block set apart with ours above
theirs and the base folded beneath. The developer takes ours, theirs, or both for a block, edits
anywhere, and saves: the file is resolved and the editor closes, back to the list of conflicts.
Leaving with a result not saved asks first. The editor is the same in the desktop application,
the web interface of the remote server, and the terminal application.

## Existing behaviour

- The stopped operation's panel in the web interface and its dialog in the terminal application
  list the conflicted files, each resolved by ours, theirs, or as edited; these stay as they are,
  and the merge editor is offered beside them.
- Resolving as edited is refused while a marker line remains, and every resolution is refused
  where the conflict is no longer as shown. The git adapter's history tests pin both, comparing
  each way of resolving with `git checkout --ours/--theirs` and `git add`; saving from the merge
  editor goes the same way and its tests join them.
- Taking a side asks first, since it discards what the developer may have edited; leaving the
  merge editor with a result not saved is a second such question, asked by the front end.
- Greenfield otherwise: no component of either front end edits text today.

## Goals served

- G-15 — the merge editor.
- G-18 — in each front end.
- G-14 — resolving a conflict, continued or aborted after.
- Q-7 — a result not saved is not left by accident.
- Q-4 — no merge tool or editor program the configuration names runs.
- Q-5 — through the remote server, saving is a write like any other.
- Q-3 — the WebView's memory, with CodeMirror loaded.

## User stories

1. As a developer whose merge stopped on a conflict, I want to open a conflicted file in Ocelli
   and see its blocks set apart, so that I see what each side changed without another tool.
2. As that developer, I want to take ours, theirs, or both for one block with one action, so that
   a file with many blocks is resolved quickly.
3. As that developer, I want to edit the result freely, the blocks included, with undo, so that a
   block that needs a mix of both sides is resolved in place.
4. As that developer, I want to open the base of a block, so that I see what both sides changed
   from, whether or not my `merge.conflictStyle` writes it into the file.
5. As that developer, I want saving to resolve the file and take me back to the list, so that I
   continue the operation once the list is empty.
6. As that developer, I want Ocelli to refuse a save that still holds a marker line, or a file
   another editor changed since I opened it, so that a half-resolved or overwritten file is never
   staged.
7. As that developer, I want to be asked before a result I have not saved is thrown away, so that
   closing the editor by mistake loses nothing.
8. As a developer in a terminal, I want the same editor with keys that do not clash with typing,
   so that I resolve conflicts over SSH as I do on the desktop.
9. As a developer using the remote server, I want the same editor in the browser, saving only
   where the server allows writes, so that a read-only server stays read-only.
10. As a developer with a conflict that is not text on both sides, a deleted side, a link, a
    submodule, or a binary file, I want to be offered ours and theirs only, so that the editor never
    opens on something it cannot show.

## Implementation decisions

- The core finds a conflicted file's blocks once for every front end: from its marker lines in
  the styles `merge`, `diff3`, and `zdiff3`, each block's base from the file's base section where
  it has one, else from a merge of the index's three stages in memory in the `diff3` style where
  its block holds the same ours and theirs. It also says whether the editor may open the file.
- The repository port gains a read of a conflicted file's text with its blob id and its blocks,
  and the writer a resolution that takes the edited result: refused while a marker line remains,
  or where the working directory's file or the conflict is no longer as read; else the index gets
  what `git add` stages for the file holding the result, and the working directory the result
  through libgit2's checkout, its line endings kept.
- The shared commands carry both, so the desktop host and the remote server answer them alike;
  the remote server performs the save only where writes are allowed.
- The web interface opens the editor as an overlay over the commit graph, as the diff does, from
  an "Edit…" action of each file in the stopped operation's panel; CodeMirror 6 with its state,
  view, and commands packages only, the blocks drawn as decorations in the theme's tokens, each
  block with its actions, the base folded; saving resolves and closes; leaving with a result not
  saved asks first.
- The terminal application opens it full screen with `m` from the conflict dialog, on
  `tui-textarea-2`: Alt-o, Alt-t, Alt-b take ours, theirs, or both for the block under the cursor,
  Alt-n and Alt-p move between blocks, Alt-a opens and closes the base, Ctrl-S saves, Esc leaves,
  asking first where the result is not saved.
- The writer resolves through the merge editor only where the core says the editor may open the
  file; every other conflict keeps ours, theirs, and as edited.

## Testing decisions

- The core's tests of the blocks copy the style of its layout and search tests: plain values in,
  values out, one case per marker style, a file edited since, a base from the stages, each kind of
  file the editor does not open, and each block action.
- The git adapter's tests of saving copy the resolution tests of the history tests: a repository
  made with `git`, the save compared with `git add` of the same file, the refusals leaving
  nothing changed.
- The shared commands and the remote server copy the write tests of the remote server: a save in
  a tab of a server started with writes allowed and without.
- The web interface's tests copy the pure-function tests beside its components: the decorations
  and actions computed from the blocks, and the question asked before leaving; the overlay's
  drawing is checked by hand in the desktop application.
- The terminal application's tests copy its key-handling tests: each key's action on a fixture of
  blocks, saving, and the question before leaving.

## Out of scope

- A merge editor for a conflict that is not text on both sides.
- Highlighting a language in the editor.
- A limit on the size of a file the editor opens.
- Resolving several files in one editor, or moving to the next file on save.
- `git rerere`, and any merge tool the configuration names.

## ADR impact

- [ADR-0031](../../adr/0031-writes-what-git-writes.md) decides the merge editor; this plan
  follows its section on it and needs no new ADR.
- [ADR-0033](../../adr/0033-discarding-work-only-after-confirmation.md): leaving a result not
  saved is asked by the front end, as ADR-0031 states; no write joins the core's list.
- [ADR-0035](../../adr/0035-remote-server-websocket-per-tab.md) and
  [ADR-0037](../../adr/0037-git-adapter-confined-to-the-server-root.md) bind the read and the save
  through the remote server.
- [ADR-0026](../../adr/0026-diff-overlay-and-file-list.md) for the overlay,
  [ADR-0024](../../adr/0024-themes-tokens-and-palettes.md) for the tokens,
  [ADR-0029](../../adr/0029-terminal-application-ratatui.md) for the terminal, and
  [ADR-0007](../../adr/0007-toolchain-rust-and-typescript.md) for the two new dependencies.
