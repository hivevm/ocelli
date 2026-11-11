# ADR-0025: A conflicted file's text is resolved in a merge editor of Ocelli's own, one column of blocks the core finds, whose saved result the writer takes as the developer's edit

- **Status:** 🟢 accepted
- **Date:** 2026-10-31
- **Deciders:** Markus Brigl
- **Applies to:** the blocks the core finds in a conflicted file in `crates/core/src/merge.rs`, the merge editor's read and save in `crates/core/src/port.rs` and `crates/git/src/conflicts.rs`, the merge editor of the web interface in `src-web/src/MergeEditor.svelte`, `src-web/src/mergeEditor.ts`, `src-web/src/mergeBlocks.ts`, and `src-web/src/App.svelte`, the merge editor of the terminal application in `crates/tui/src/merge.rs`, `crates/tui/src/app.rs`, and `crates/tui/src/ui.rs`, the fixture `crates/core/tests/fixtures/merge.json`, the tests of `crates/git/tests/merge_editor.rs` and `crates/server/tests/merge_editor.rs`, and the dependencies on CodeMirror and `tui-textarea-2` in `src-web/package.json` and `crates/tui/Cargo.toml`

## Context

G-15 asks for a merge editor: the developer opens a conflicted file, sees both sides and the
result, edits the result, and Ocelli takes it as the file's resolution. G-18 asks for it in each
front end, the terminal application on an 80-column terminal included. Saving a result is a write,
and every write does what `git` does and runs no program
([ADR-0023](0023-writes-what-git-writes.md)): `git mergetool` and the merge tool the configuration
names are programs the repository can choose (Q-4).

A conflicted file is written by libgit2 with `git`'s markers, in the style `merge.conflictStyle`
names, so it holds the base between `|||||||` and `=======` only under `diff3` or `zdiff3`, under
the labels [ADR-0024](0024-conflict-labels-path-safe.md) decides; the developer may have edited it
in an editor of their own before opening it in Ocelli. Not every conflict is text on both sides:
a side may be deleted, a link, or a submodule, and a file may be binary.

A text editor is a component neither front end has: the web interface runs in a WebView whose
style sources allow inline styles ([ADR-0034](0034-no-egress-hardened-webview.md)), the terminal
application draws with ratatui 0.30 ([ADR-0031](0031-terminal-application-ratatui.md)), and every
component counts against the memory Q-3 allows. Once the developer edits, the text can lose a
marker line, gain one, or hold a run of marker lines typed or pasted, and both front ends must
agree on what is still a block. Discarding a result not saved loses work (Q-7).

## Decision

We will resolve a conflicted file's text in a merge editor of Ocelli's own, one column holding
the blocks the core finds in the file as the developer left it, whose saved result the writer
takes as the developer's edit of the file.

- **Where it opens.** On a conflicted file whose sides ours and theirs both hold text: neither
  side deleted, a link, or a submodule, and neither holding a NUL byte or anything but UTF-8.
  Any other conflict is resolved by ours, theirs, or as edited alone
  ([ADR-0023](0023-writes-what-git-writes.md)).
- **What it starts from.** The working directory's file as it is, edits made in another editor
  included, and its blob id. The core finds its blocks, once for every front end: each run from
  a `<<<<<<<` line to its `>>>>>>>` line, ours before `=======` and theirs after it. A block's
  base is the file's `|||||||` section where it has one; else, where the index holds a base, the
  bases of the first run, one or more blocks in a row, of a merge of the three stages in memory
  in the `diff3` style whose ours and theirs, joined by the lines between them and without the
  lines both sides share at its ends, are the block's, joined by the same lines; else the block
  has none. Blocks take the runs in their order, each run once. The `merge` style `git` writes by
  default moves those shared lines out of a block and joins conflicts close together into one,
  which the `diff3` style does not.
- **What is a block after an edit.** Both front ends hold the blocks the core found when the
  file was read, each block's marker lines followed through every edit. A block an edit took a
  marker line from, or moved one out of order, is no block, and a run of marker lines the
  developer types or pastes is none either. The fixture `crates/core/tests/fixtures/merge.json`
  holds the cases both front ends' tests read.
- **What it shows.** One column at any width: the file's text with each block set apart, ours
  above theirs, its base folded until the developer opens it. No language is highlighted. The
  terminal application, whose text widget folds no lines, sets a base section the file holds
  aside when the editor opens and shows it, as a base found in memory, beneath the text while
  open; no line outside a base section is changed, and no saved result could keep one, since it
  holds a marker line.
- **What it does.** A block is replaced by ours, by theirs, or by both, ours first; the text is
  editable everywhere, the blocks included, with undo.
- **Saving** takes the result as the file's resolution, refused while it holds a marker line, as
  as edited is, and refused, with nothing changed, where the working directory's file is no
  longer the one opened or the conflict no longer as shown. The index gets what `git add` stages
  for the file holding the result, and the working directory the result through libgit2's
  checkout, its line endings as the file had them. It is a write like any other: through the
  remote server only where writes are allowed.
- **Leaving** the merge editor with a result not saved discards it, so the front end asks the
  developer first (Q-7).
- **Components.** The web interface edits with CodeMirror 6, `@codemirror/state`,
  `@codemirror/view`, and `@codemirror/commands` alone as its direct dependencies, the blocks
  drawn as its decorations; what `@codemirror/commands` brings in as its own is allowed. The
  terminal application edits with `tui-textarea-2` on ratatui 0.30.

**Out of scope:** a merge editor for a conflict that is not text on both sides; highlighting a
language in the merge editor; which writes discard work and how they are confirmed
([ADR-0026](0026-discarding-work-only-after-confirmation.md)).

## Alternatives considered

- **The configured merge tool** — the developer's own, but a program the configuration names
  (Q-4).
- **No merge editor, an edit in the developer's own editor taken as edited** — what the record
  chose until 2026-10-31, no component of Ocelli's own for what a text editor and one of the two
  sides already do; G-15 asks for the editor in Ocelli, in every front end.
- **Three columns, ours, the result, and theirs, or both sides above the result** — the layout of
  most merge editors, but neither fits an 80-column terminal, and a front end that stacks them
  there is two layouts to keep in step.
- **Editing the whole file only, sides taken per file** — less to build, but a file with several
  blocks is resolved by hand where one key would take a side.
- **Starting from a merge of the index's stages** — a base for every block, but it drops what the
  developer edited in another editor, and saving would discard it.
- **Finding the blocks anew on every edit** — what the terminal application first did: no
  markers to follow, but a run of marker lines the developer types or pastes becomes a block, and
  the two front ends disagree on what is a block.
- **The base always shown, or never** — always is three parts per block on a narrow terminal;
  never hides what both sides changed from.
- **Opening a delete or modify conflict with the deleted side empty** — one editor for more
  conflicts, but the choice is to keep the file or not, which ours or theirs makes.
- **A limit on the size of a file the editor opens** — keeps a huge file from stalling a front
  end, but measured on the largest files at hand no interaction comes near stalling one
  (*Consequences*), so a limit would refuse files the editor handles.
- **Monaco** — the most complete editor, but several megabytes in the WebView (Q-3).
- **CodeMirror's `@codemirror/merge`, or its `codemirror` bundle** — a merge view ready to use,
  but one of two documents, not a conflict of three; the bundle brings completion, linting, and
  languages the editor does not use.
- **An editor component of Ocelli's own in the web interface or the terminal** — no dependency,
  but undo, selection, and cursor movement written twice.
- **`tui-textarea` 0.7.0** — the widget most used, but it needs ratatui 0.29, whose types do not
  meet 0.30's, and has had no release since October 2024.

## Sources / Prior art

- `git merge-file`, `merge.conflictStyle`, and the `diff3` and `zdiff3` styles —
  <https://git-scm.com/docs/git-merge-file>, <https://git-scm.com/docs/git-config>; libgit2's
  `git_merge_file_from_index` and its `GIT_MERGE_FILE_STYLE_DIFF3` —
  <https://libgit2.org/libgit2/>
- CodeMirror 6, `@codemirror/view` 6.43.15, `@codemirror/state` 6.7.6, and `@codemirror/merge`
  6.12.3, MIT, from the npm registry on 2026-10-31 — <https://codemirror.net/docs/>; its styles
  are mounted at run time by `style-mod`, which the WebView's `style-src 'self' 'unsafe-inline'`
  allows.
- What `@codemirror/commands` 6.11.1 brings in, as `bun.lock` held it on 2026-11-11:
  `@codemirror/language` 6.13.1, `@codemirror/streamparser` 6.0.0, `@lezer/common` 1.5.5,
  `@lezer/highlight` 1.2.5, and `@lezer/lr` 1.4.12, and below them and the direct dependencies
  `style-mod` 4.1.4, `crelt` 1.0.7, `w3c-keyname` 2.2.8, and `@marijn/find-cluster-break` 1.0.4.
- `tui-textarea` 0.7.0, its dependency `ratatui ^0.29.0`, and `tui-textarea-2` 0.13.2, MIT, on
  `ratatui-core ^0.1.0` and `ratatui-widgets ^0.3.0` with `crossterm ^0.29`, from crates.io on
  2026-10-31 — <https://crates.io/crates/tui-textarea-2>
- The merge editors of VS Code and IntelliJ IDEA, three columns each.

## Consequences

- Positive: a conflict is resolved in Ocelli, block by block or by hand, in every front end
  (G-15, G-18), from the file as the developer left it; both front ends agree on what is a block
  after any edit; a result not saved is not lost by accident (Q-7).
- Negative / trade-offs: three direct dependencies of the web interface, with what they bring in,
  and one of the terminal application, a fork with fewer users than the widget it forks; a binary
  file or one side deleted is still resolved by ours or theirs; a large conflicted file is opened
  whole, which Q-2 and Q-3 do not measure. Measured in release builds on 2026-11-03 on a
  70,004-line file with one block and a 35,009-line file with 3,001 blocks: the core reads the
  file for the editor in 92 / 137 ms (its bases 13 / 123 ms); the terminal application on 80×24
  answers a key with its frame in about 26 / 9 ms and takes a side in 3 / 2 ms; the WebView opens
  the file in 262 / 431 ms, adding 180 / 210 MB, takes a side in about 30 / 35 ms, and answers a
  key in about 2 / 15 ms. No interaction exceeds 50 ms, so no limit is set.
- Follow-ups: highlighting a language in the merge editor; a limit on the size of a file it opens,
  should a larger file stall a front end.

## Enforcement

The core's tests of the merge editor cite this ADR and G-15: blocks found in the `merge`, `diff3`,
and `zdiff3` styles and in a file edited since, a base from the stages where the file holds none,
a binary file, a link, a submodule, and a deleted side offered no editor, each block action; a
core test writes `crates/core/tests/fixtures/merge.json`, the text a side taken leaves and the
blocks still drawn once a run of marker lines is pasted, and the tests of both front ends,
`crates/tui/src/merge.rs` and `src-web/src/mergeBlocks.test.ts`, read it. The git adapter's tests
save a result and compare the index and the working directory with `git add` of the same file,
and refuse one with a marker line or over a file changed since it was opened, with nothing
changed; the tests of both front ends cite it for asking before a result not saved is left.
