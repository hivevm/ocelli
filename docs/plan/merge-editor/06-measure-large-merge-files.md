# Ticket: Measure the merge editor on large files and decide on a limit

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

A measurement of the merge editor on files a developer meets, a lockfile of tens of thousands of
lines and a file of thousands of blocks: how long the core takes to find the blocks and their
bases, the web interface to open the file and to take a side, and the terminal application to
draw a frame and handle a key, which finds the blocks again on every key and frame. ADR-0031 sets
no limit on the size of a file the editor opens and says a limit can follow a measurement; this
ticket takes the measurement and brings the human what it shows, so they decide whether a limit,
or a change to how often the blocks are found, follows.

## Acceptance criteria

- [ ] Given a conflicted file of 70,000 lines with one block, and one of 3,000 blocks, some of
      them matching no block of the in-memory merge, when the core reads it, then the time to
      find its blocks and their bases is measured in a release build.
- [ ] Given the same files, when the terminal application opens one, takes a side, and draws,
      then the time of a key and of a frame is measured on an 80x24 pseudo-terminal.
- [ ] Given the same files, when the web interface opens one and takes a side, then the time is
      measured in the WebView or a browser.
- [ ] Given the measurements, when they are brought to the human, then the decision, no limit, a
      limit, or finding the blocks less often, is recorded where it binds: ADR-0031 for a limit,
      the code for the rest.

## Read first

- docs/adr/0031-writes-what-git-writes.md (The merge editor; a limit on the size of a file it
  opens, an alternative considered).
- Q-2 and Q-3 in docs/SPECIFICATION.md, and the measurement scripts and `crates/measure`.
- Golden path: the core's `blocks_with_bases` and the terminal application's `merge.rs`.
- How the terminal application was driven by hand: a pseudo-terminal of 80x24 read through a
  terminal emulator (pyte), keys written as a terminal sends them (Alt as Esc first).

## Blocked by

- None (can start immediately)

## Mode

with the human: the outcome is a decision, a limit or none, that ADR-0031 leaves to a measurement
and the human.
