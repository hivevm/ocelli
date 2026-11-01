# Ticket: The tests pass whatever the developer's global and system git configuration holds

<!-- Copy to docs/plan/<spec>/<NN>-<slug>.md, next to the spec it slices, if it has one. A
     vertical slice: a narrow but complete path through every layer the change touches, demoable
     or verifiable on its own, sized to one fresh agent session. Claim it by creating the branch
     ticket/<spec>/<NN>-<slug> BEFORE any work, so parallel sessions sharing the repository skip
     it. The branch that lands it deletes this file. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

The Rust tests read the global and system git configuration of whoever runs them: libgit2 in
the writer and reader, and the test's own `git`. Under a global `core.autocrlf=true`, which a Dev
Container takes up from the host's gitconfig, about 30 tests in `ocelli-git` (`git_parity`,
`history`, `write`, `git2_unwraps`) and `ocelli-server` (`allow_writes`, `writes`) check files out
with CRLF and fail, while CI, which has no such setting, stays green. A few tests already guard
themselves one by one: `fixture_repo` turns `core.autocrlf` off in its repository, and
`write_identity` empties libgit2's global, XDG, and system search paths.

Every test binary that runs libgit2 or the test's `git` sees no global, XDG, or system
configuration, set up in one place that the test support shares, so a new test is covered
without remembering it. The test's `git` and libgit2 see the same configuration, so a parity test
compares like with like. What a test means to configure, it writes into its repository's own
configuration.

## Acceptance criteria

- [ ] Given a global gitconfig with `core.autocrlf=true`, when `cargo test --workspace --locked`
      runs, then every test passes.
- [ ] Given a global gitconfig with a setting that changes what a write does beyond line endings,
      such as `merge.conflictStyle=diff3` or `pull.rebase=true`, when the tests run, then every
      test passes.
- [ ] Given a new test file that uses the shared test support, when it runs libgit2 or the
      test's `git`, then it sees no global or system configuration without a line of its own.
- [ ] Given the tests that check the identity, signing, and credentials a repository's
      configuration names, when they run, then they still decide what they decided before.
- [ ] Given a sensor or self-test that runs the test suite under a hostile global gitconfig,
      when a test reads the developer's configuration again, then it fails, so the isolation
      cannot quietly erode.

## Read first

- docs/adr/0002-decisions-verified-by-tests.md.
- docs/adr/0017-git2-repository-reader.md, docs/adr/0031-writes-what-git-writes.md: the parity
  of the writer with `git`, which the isolation must keep on both sides.
- docs/adr/0034-remotes-through-libgit2-transports.md: credentials from the developer's own
  setup, which the tests must not start to depend on.
- Glossary: *Write*.
- Golden path: the search paths `write_identity` empties, and the `git` the shared test support
  starts with its fixed `-c` settings.
- Seams: the test support of `ocelli-git` and `ocelli-server`. Open questions for the session:
  libgit2's search paths are process-wide, so where a binary sets them once before any test
  touches libgit2; whether the remote server's tests run libgit2 in a process of their own, which
  the process-wide setting does not reach; whether the test's `git` takes `GIT_CONFIG_GLOBAL` and
  `GIT_CONFIG_NOSYSTEM` or its `-c` settings suffice.

## Blocked by

- None (can start immediately)

## Mode

with the human: the harness ADR keeps the cap on unattended work at zero, and the change touches
the test support every Rust test shares.
