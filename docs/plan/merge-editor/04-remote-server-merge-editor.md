# Ticket: Resolve a conflicted file in the merge editor through the remote server

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

The web interface of the remote server offers the same merge editor: the read of a conflicted
file's text and blocks, and the save, travel over the tab's connection as the shared commands
answer them. A server started without writes allowed offers the editor to read but refuses the
save, saying so; one confined to its root opens nothing outside it.

## Acceptance criteria

- [ ] Given a server started with writes allowed, when a tab saves a merge editor's result, then
      the file is resolved as in ticket 01 and the tab's stopped operation shows the remaining
      conflicts.
- [ ] Given a server started without writes allowed, when a tab saves, then it is refused, nothing
      changes, and the editor says why.
- [ ] Given a conflicted file below a link that leads outside the server root, when a tab reads it,
      then the read is refused as every read outside the root is.
- [ ] Given two tabs on one repository, when one saves while the other holds the repository's turn,
      then the save waits for its turn and checks the file again after the wait.

## Read first

- docs/adr/0035-remote-server-websocket-per-tab.md,
  docs/adr/0036-remote-server-localhost-tls-session-cookie.md,
  docs/adr/0037-git-adapter-confined-to-the-server-root.md.
- docs/adr/0031-writes-what-git-writes.md (The merge editor).
- Glossary: *remote server*, *server root*, *turn*, *confined*.
- Golden path: the remote server's write tests in a tab of a server with and without writes.
- Seams: the remote server's tests over its WebSocket.

## Blocked by

- docs/plan/merge-editor/01-web-merge-editor-tracer.md

## Mode

with the human: the harness ADR keeps the cap on unattended work at zero, and the remote server is
a trust boundary (Q-5).
