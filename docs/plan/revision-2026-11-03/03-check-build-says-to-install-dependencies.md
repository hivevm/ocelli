# Ticket: The build gate says to install the web interface's dependencies where they are missing

<!-- Filed by the design revision of 2026-11-03. Decided in docs/adr/0004-feature-layer.md. -->

## What to build

During ticket merge-editor/05, `scripts/check-all.sh` in a fresh worktree failed with "vite: command
not found": a worktree shares no `node_modules/`, and `scripts/check-build.sh` expects the
dependencies installed without saying so where they are not. `bun install --frozen-lockfile` then
reported "no changes" without creating `node_modules/`; `--force` made it. Each fresh worktree,
which `work-tickets` makes for every ticket, meets this again and spends a run of the whole gate on
it.

`scripts/check-build.sh` looks before its first command: where `node_modules/` is missing, or does
not hold a package `bun.lock` locks, it reports "Build (web interface)" and the web interface's test
and lint failed with one line naming the command that installs them, and skips those three, while
the Rust commands run on. Whether it looks at the directory, at a package `src-web/package.json`
names directly, or asks bun, is read off bun's documentation and bun run on the case, not recalled
([`AGENTS.md` §4](../../../AGENTS.md#4-working-style)); the `--force` above is part of that
question.

## Acceptance criteria

- [ ] Given a worktree without `node_modules/`, when `scripts/check-build.sh` runs, then it fails
      with a line that names the install command, and does not print "vite: command not found".
- [ ] Given a worktree with the dependencies installed, when it runs, then it runs every command as
      before.
- [ ] The self-test covers both verdicts.

## Read first

- `scripts/check-build.sh`, `scripts/check-all.sh`, README.md → "Build, Test & Run"
- `.agents/skills/work-tickets/SKILL.md` (where the worktrees are made)
- docs/CONVENTIONS.md, "A failure is reported, never dropped or read as a value" and "A self-test
  asserts what it builds"

## Blocked by

- None (can start immediately)

## Mode

with the human — how bun tells an installed tree from a stale one is not yet verified, and the build
gate is what every agent hits ([`AGENTS.md` §6](../../../AGENTS.md#6-project-rules)).
