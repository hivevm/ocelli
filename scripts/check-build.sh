#!/usr/bin/env bash
#
# Build, test, and lint gate for this repository.
#
# AGENTS.md §5 makes a change done only when it builds, its tests pass, and linters and formatters
# are clean. The commands below are the ones ADR-0007 names and README.md → "Build, Test & Run"
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
# The Rust tests run with a developer's git configuration in reach that changes what a write does,
# so a test that reads it again fails here rather than only on a machine whose configuration
# differs. libgit2 reads it at the XDG level; a global ~/.gitconfig still overrides its keys, so
# CI, which has none of them, is where this holds fully. A copy in a directory of its own, since
# whatever else reads XDG_CONFIG_HOME may write there.
developer_config="$(mktemp -d)"
trap 'rm -rf "$developer_config"' EXIT
mkdir -p "$developer_config/git"
cp "$ROOT/crates/git/tests/developer-config/git/config" "$developer_config/git/config"
run "Test (Rust)" env XDG_CONFIG_HOME="$developer_config" cargo test --workspace --locked
run "Test (web interface)" bun test src-web
run "Format check" cargo fmt --all --check
run "Lint (clippy)" cargo clippy --workspace --all-targets --locked -- -D warnings
run "Lint (web interface)" bun run --cwd src-web lint

# The Windows build's lint, where cargo-xwin and its LLVM tools are installed: code under
# cfg(windows), or a test whose helper only unix uses, fails only there, and CI's Windows runner
# would be the first to say so. The desktop host is left out: its resource compiler needs the
# Windows SDK's own tools. Where a tool is missing the step says so and goes on; CI's Windows
# runner lints that build natively.
printf '\n==> %s\n' "Lint (clippy, Windows, through cargo-xwin)"
missing=()
for tool in cargo-xwin llvm-lib clang-cl; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-msvc ||
  missing+=("the x86_64-pc-windows-msvc target")
if ((${#missing[@]})); then
  echo "Skipped: ${missing[*]} not installed."
else
  cargo xwin clippy --workspace --exclude ocelli --all-targets --locked \
    --target x86_64-pc-windows-msvc -- -D warnings || failed+=("Lint (clippy, Windows)")
fi

printf '\n'
if ((${#failed[@]})); then
  echo "FAILED:" >&2
  printf '  - %s\n' "${failed[@]}" >&2
  exit 1
fi
echo "Build, test and lint passed."
exit 0
