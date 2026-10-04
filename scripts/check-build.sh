#!/usr/bin/env bash
#
# Build, test, and lint gate for this repository.
#
# AGENTS.md §5 makes a change done only when it builds, its tests pass, and linters and formatters
# are clean. The commands below are the ones ADR-0008 names and README.md → "Build, Test & Run"
# lists; a command changed there is changed here in the same change.
#
# This script exists so that the local gate and CI run one list instead of two. The build job of
# .github/workflows/checks.yml sets up the toolchain and then calls this script, and
# scripts/check-all.sh calls it too — commands written inline in the workflow would run in CI and
# never before a push, which is what check 9 of scripts/check-docs.sh refuses.
#
# The full desktop bundle ('bun run tauri build') is deliberately not here: it is a release step,
# needs the bundler toolchain of every target platform, and says nothing a per-change gate needs.
#
# Dependencies are expected to be installed ('bun install --frozen-lockfile'); this script builds
# and tests, it does not provision. Every command runs even after an earlier one fails, so one run
# reports everything that is wrong.
#
# Usage:
#     scripts/check-build.sh        (or: bash scripts/check-build.sh)
# Exit code 0 when every command passes, 1 when any of them fails.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 1

failed=()

# run <label> <command...> — run one command, recording a failure without aborting the run.
run() {
  local label="$1"
  shift
  printf '\n==> %s\n' "$label"
  "$@" || failed+=("$label")
}

run "Build (Rust)" cargo build --workspace --locked
run "Build (web interface)" bun run --cwd src-web build
run "Test (Rust)" cargo test --workspace --locked
run "Test (web interface)" bun test src-web
run "Format check" cargo fmt --all --check
run "Lint (clippy)" cargo clippy --workspace --all-targets --locked -- -D warnings
run "Lint (web interface)" bun run --cwd src-web lint

printf '\n'
if ((${#failed[@]})); then
  echo "FAILED:" >&2
  printf '  - %s\n' "${failed[@]}" >&2
  exit 1
fi
echo "Build, test and lint passed."
exit 0
