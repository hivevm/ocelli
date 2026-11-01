# Ticket: Say in the browser why the merge editor opens no file

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

In a tab of the remote server, when the merge editor opens no file, the tab says why as the
desktop application does: a side deleted, a link, a submodule, or a file that holds no text, or
the file no longer conflicted or no longer a file. Today every such reason is a refusal of the git
adapter, which the server scrubs to "Reading the repository failed" because it may name a path on
the server. The editor's reasons name only the conflicted file's path below the repository, so
they can reach the browser as Ocelli's own refusals do, while what git itself says stays scrubbed.

## Acceptance criteria

- [ ] Given a tab on a server, when it reads a conflicted file the editor does not open (a side
      deleted, a binary file, a link, a submodule), then it gets the reason the desktop host
      gets for the same file, naming the file's path below the repository and no path on the
      server.
- [ ] Given a read that fails for any other reason, such as a path outside the server root or
      what git says, when the tab gets the answer, then it is still "Reading the repository
      failed".
- [ ] Given a refused read in a tab, when the web interface shows it, then the reason stands
      where the editor would have opened, as the desktop application shows it.

## Read first

- docs/adr/0037-git-adapter-confined-to-the-server-root.md (what reaches the browser, scrubbed).
- docs/adr/0035-remote-server-websocket-per-tab.md.
- docs/adr/0031-writes-what-git-writes.md (The merge editor: where it opens).
- Glossary: *remote server*, *server root*, *confined*.
- Golden path: the kinds of `HostError` in the shared commands and `scrub` in the remote server.
- Seams: the remote server's tests over its WebSocket (`crates/server/tests/merge_editor.rs`), the
  hostile-commands test.

## Blocked by

- None (can start immediately)

## Mode

with the human: the remote server is a trust boundary (Q-5), and what reaches the browser is
ADR-0037's to decide.
