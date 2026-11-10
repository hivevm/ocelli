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
# and tests, it does not provision. Where a direct dependency bun.lock names for a workspace is
# missing from that workspace's node_modules/, or is there at another version than bun.lock locks,
# as in a fresh worktree, which shares no node_modules/, the web interface's build, test, and lint
# are not run but reported failed, under one line naming the command that installs them; bun has
# no command that tells this without installing. Every other command runs even after an earlier
# one fails, so one run reports everything that is wrong.
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

# web_unmet — the first direct dependency of a workspace in bun.lock that is not installed at the
# version bun.lock locks, as its path and what was found there; nothing where each is. A package is
# looked for as Node resolves it, in the workspace's node_modules/ and then the root's, so both of
# bun's linkers, isolated and hoisted, are read. An optional or a peer dependency is not read: one
# may be missing by design. A dependency bun.lock nests under the workspace's name is read at that
# version. A bun.lock that names no direct dependency this reads, or none at all, is reported too:
# a layout this does not know is no pass.
web_unmet() {
  local listed dir name want pkg got
  listed="$(awk '
    /^  "workspaces": \{/ { section = "workspaces"; next }
    /^  "packages": \{/ { section = "packages"; next }
    /^  [^ ]/ { section = "" }
    section == "workspaces" && /^    "[^"]*": \{/ { split($0, q, "\""); dir = q[2]; kind = ""; next }
    section == "workspaces" && /^      "name": "/ { split($0, q, "\""); named[dir] = q[4]; next }
    section == "workspaces" && /^      "/ { split($0, q, "\""); kind = q[2]; next }
    section == "workspaces" && /^        "/ && (kind == "dependencies" || kind == "devDependencies") {
      split($0, q, "\""); n++; dirs[n] = dir; names[n] = q[2]; next
    }
    section == "packages" && /^    "[^"]*": \["/ {
      split($0, q, "\""); match(q[4], /@[^@]*$/); locked[q[2]] = substr(q[4], RSTART + 1)
    }
    END {
      for (i = 1; i <= n; i++) {
        nested = named[dirs[i]] "/" names[i]
        printf "%s|%s|%s\n", dirs[i], names[i], (nested in locked) ? locked[nested] : locked[names[i]]
      }
    }' bun.lock 2>&1)" || listed=""
  if [[ -z "$listed" ]]; then
    echo "bun.lock names no direct dependency this gate can read"
    return
  fi
  while IFS='|' read -r dir name want; do
    pkg="${dir:+$dir/}node_modules/$name/package.json"
    [[ -f "$pkg" || -z "$dir" ]] || pkg="node_modules/$name/package.json"
    if [[ ! -f "$pkg" ]]; then
      echo "${dir:+$dir/}node_modules/$name is missing"
      return
    fi
    got="$(grep -o -m1 '"version" *: *"[^"]*"' "$pkg" | sed 's/.*"\([^"]*\)"$/\1/')"
    if [[ -z "$got" ]]; then
      echo "${pkg%/package.json} has no version"
      return
    elif [[ "$got" != "$want" ]]; then
      echo "${pkg%/package.json} is $got, bun.lock locks ${want:-none}"
      return
    fi
  done <<< "$listed"
}

unmet="$(web_unmet)"
if [[ -n "$unmet" ]]; then
  printf "\nThe dependencies are not installed as bun.lock locks them (%s): run 'bun install --frozen-lockfile'.\n" "$unmet"
fi

# web <label> <command...> — run one of the web interface's commands, or, where its dependencies
# are not installed, report it failed without running it.
web() {
  if [[ -n "$unmet" ]]; then
    printf '\n==> %s\n%s\n' "$1" "Not run: the dependencies are not installed (above)."
    failed+=("$1")
  else
    run "$@"
  fi
}

run "Build (Rust)" cargo build --workspace --locked
web "Build (web interface)" bun run --cwd src-web build
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
web "Test (web interface)" bun test src-web
run "Format check" cargo fmt --all --check
run "Lint (clippy)" cargo clippy --workspace --all-targets --locked -- -D warnings
web "Lint (web interface)" bun run --cwd src-web lint

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
grep -qx x86_64-pc-windows-msvc <<< "$(rustup target list --installed 2>/dev/null)" ||
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
