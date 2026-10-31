# Ticket: Edit a conflicted file in the desktop application and save it as its resolution

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

The tracer bullet through every layer. In the desktop application, a merge stopped on a conflict
shows an "Edit…" action beside each conflicted file whose sides are both text. It opens the file
as it lies in the working directory in a merge editor over the commit graph, as the diff opens;
the text is editable with undo, the blocks marked by their marker lines. Saving resolves the file
as an edit and closes the editor, back to the stopped operation's panel; a save that still holds a
marker line, or over a file changed since it was opened, is refused with nothing changed. Leaving
with a result not saved asks first. Block actions and the base come in the next ticket.

## Acceptance criteria

- [ ] Given a conflicted file in the `merge`, `diff3`, or `zdiff3` style, when the core reads it,
      then it returns the file's text, its blob id, and its blocks with ours, theirs, and, where the
      file holds one, the base.
- [ ] Given a conflict whose side is deleted, a link, a submodule, or a file with a NUL byte or
      anything but UTF-8, when the core reads it, then it says the editor does not open it, and the
      panel offers no "Edit…".
- [ ] Given an edited result without marker lines, when it is saved, then the index and the
      working directory are what `git add` leaves for the file holding the result, line endings
      kept, and the file is no longer conflicted.
- [ ] Given a result that holds a marker line, or a working-directory file changed since it was
      opened, or a conflict no longer as read, when it is saved, then it is refused and nothing
      changes.
- [ ] Given an editor with a result not saved, when the developer leaves it, then the front end
      asks first, and leaving without confirming keeps the editor open.
- [ ] CodeMirror 6 enters as `@codemirror/state`, `@codemirror/view`, and `@codemirror/commands`
      only, pinned in the lockfile, and the WebView's style sources allow it unchanged.

## Read first

- docs/adr/0031-writes-what-git-writes.md (The merge editor, A stopped operation), with
  docs/adr/0032-conflict-labels-path-safe.md for the labels in the markers.
- docs/adr/0008-ports-and-adapters.md and docs/adr/0017-git2-repository-reader.md for the port.
- docs/adr/0022-desktop-host-answers-what-the-core-decides.md,
  docs/adr/0023-svelte-web-interface.md, docs/adr/0025-no-egress-hardened-webview.md,
  docs/adr/0026-diff-overlay-and-file-list.md.
- docs/adr/0033-discarding-work-only-after-confirmation.md for the question before leaving.
- Glossary: *conflict*, *operation in progress*, *write*, *departure*.
- Golden path: the resolution of a conflicted file as edited, from the panel through the shared
  commands and the writer; the diff overlay for the editor's place.
- Seams: the core's tests of the blocks, the git adapter's history tests, the web interface's
  pure-function tests beside its components.

## Blocked by

- None (can start immediately)

## Mode

with the human: the harness ADR keeps the cap on unattended work at zero, and the editor's look
in the desktop application is checked by hand.
