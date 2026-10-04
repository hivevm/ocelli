# ADR-0020: A diff covers only the commit graph, and the changed files show as a tree or a flat list

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** the diff overlay and the changed-files list in `src-web/src/` and `crates/tui/src/`

## Context

When the developer selects a commit, Ocelli shows its changed files, and the diff of a changed file
on request (G-12). In practice that is a loop, not a single view: **pick a file, read it, pick the
next.** Every layout question here follows from that loop.

Two forces bear on it. **Where the diff appears:** a panel pinned to the bottom of the window,
spanning its full width at a fixed height, splits attention between two areas and covers the list
of changed files, so that moving to the next file means closing the diff first; the loop breaks at
its most frequent step. **How the list is shown:** a flat, alphabetical list is hard to scan for a
commit that touches many files in deep directories, since paths share long prefixes and the list
gives no sense of where a change is concentrated; a tree shows that structure, but a flat list is
faster to scan for small changes, which are the common case.

The same loop has to work in the web interface and in the terminal application (G-8), whose
runtimes offer different means: the one a DOM and a canvas, the other a grid of cells.

## Decision

We will keep the list of changed files reachable at all times, by drawing the diff over the commit
graph only and offering the list as a tree or a flat list, switchable at any time.

- **The diff covers only the commit graph.** In the web interface it is rendered inside the
  canvas container, which is `position: relative`, filling it with `position: absolute; inset: 0`;
  the sidebar with the changed files stays visible and usable. Selecting a file opens the diff
  over the commit graph, the list stays usable so the next file is picked directly, and closing
  the diff reveals the commit graph again.
- **The terminal application keeps the same rule with what its runtime offers.** It draws the
  diff over the graph pane only, after the file list has been drawn. The list keeps its place and
  its keys while the diff is open; `t` switches the view and `Esc` closes the diff.
- **The file list has two interchangeable views**, switched by two buttons in the section's header
  or by `t` in the terminal, held in state as `'flat' | 'tree'`, with `'tree'` the default in both
  front ends. The flat view is one alphabetical level. The tree view is derived from the flat list,
  drawn recursively with collapsible folders; chains of single-child folders collapse into one
  (`src/web` rather than `src` › `web`), each folder carries a recursive count of the changed
  files below it, and the tree is rebuilt when the selected commit changes.

**Out of scope:** the diff's own layout, unified or side by side; editing a file, which the
specification's Non-Goals exclude except to resolve a conflict.

## Alternatives considered

*Where the diff appears:*

- **A fixed panel at the bottom** — familiar from terminal tools, but it wastes width, fixes the
  height, and covers the file list, the one thing the loop needs.
- **A separate diff window** — all the room for the diff, but window management becomes the
  developer's problem and the link to the selected commit is lost; a terminal has no windows.
- **Splitting the graph area, commit graph above and diff below** — keeps both visible, but at
  common window sizes neither gets enough room, and the commit graph is not what the developer
  reads while a diff is open.

*How the list is shown:*

- **Tree only** — one code path, but worse than a flat list for the common small change.
- **Flat only, with the common prefix stripped** — cheap, but no structure, and it breaks down
  once a change spans several top-level directories.
- **One heading per directory, without nesting** — simpler than a tree, but deep hierarchies yield
  almost as many headings as files.

## Sources / Prior art

- GitKraken and Sourcetree, where the diff takes over the main area while the file list stays in a
  side panel — <https://www.gitkraken.com/>
- GitHub's pull request view "Files changed", with the same flat and tree toggle.
- VS Code's `explorer.compactFolders`, the model for collapsing single-child folders —
  <https://code.visualstudio.com/docs/getstarted/settings>
- MDN, *CSS position* and containing blocks —
  <https://developer.mozilla.org/en-US/docs/Web/CSS/position>
- GitGraph's `docs/adr/0015-diff-overlay-covers-canvas.md` on `main`, where this layout runs.

## Consequences

- Positive: the loop works, the next file is one selection away while a diff is open; the diff
  gets the full height of the graph area; no full-width panel covers unrelated parts; the tree
  makes large changes navigable and its counts show where a change is concentrated, while the flat
  view stays for those who prefer it.
- Negative / trade-offs: the commit graph is hidden while a diff is open, acceptable because the
  sidebar still shows the selected commit; the markup of a file row exists twice, in the flat list
  and the recursive tree, and the two must be kept alike; the recursive view adds some complexity.
- Follow-ups: none.

## Enforcement

The tests of `src-web/src/fileTree.test.ts` and `src-web/src/diffSplit.test.ts`, and of
`crates/tui/src/files.rs` and `crates/tui/src/diffsplit.rs`, cite this ADR with a `Verifies:`
marker: the tree is derived from the flat list, chains of single-child folders collapse, each
folder carries a recursive count, and the two views hold the same files.

`crates/tui/src/ui.rs` cites it for the rule itself, in the one runtime where a test can decide
it: it draws a loaded repository with a diff open and asserts that a graph row is gone from the
screen while the file list's own pane title is still on it, in tree view. Covering the file list
too, or starting in the flat view, fails it.

**Not mechanically decidable:** that the web interface's overlay covers the canvas and leaves the
list usable. That is layout in a browser, and a test of the CSS would restate the rule rather than
decide it; the terminal carries the rule for both.
