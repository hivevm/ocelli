#!/usr/bin/env bash
#
# Self-test of scripts/check-devcontainer-toolchain.sh. Verifies: ADR-0011
#
# Each case builds a fixture under a temporary directory, a devcontainer.json, a Dockerfile, and a
# rust-toolchain.toml as this repository has them, runs the check against it, and asserts the exit
# code and, for a failing case, the message that names the violation. The baseline passes; every
# other case breaks one thing ADR-0011 decides.
#
# Usage:
#     scripts/test-check-devcontainer-toolchain.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check-devcontainer-toolchain.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failed=0
passed=0

# fixture <name> — a directory with the three files as the baseline has them.
fixture() {
  local dir="$work/$1"
  mkdir -p "$dir/.devcontainer"
  cat > "$dir/.devcontainer/devcontainer.json" <<'JSON'
{
  "name": "fixture",
  // Built from the Dockerfile (ADR-0011).
  "build": {
    "dockerfile": "Dockerfile"
  },
  "features": {
    "ghcr.io/devcontainers/features/rust:1": {},
    "ghcr.io/devcontainers-extra/features/bun:1": { "version": "1.4.1" }
  }
}
JSON
  cat > "$dir/.devcontainer/Dockerfile" <<'DOCKER'
# syntax=docker/dockerfile:1
FROM mcr.microsoft.com/devcontainers/base:debian AS base
RUN apt-get update && apt-get install -y --no-install-recommends libwebkit2gtk-4.1-dev
DOCKER
  printf '[toolchain]\nchannel = "1.96.0"\n' > "$dir/rust-toolchain.toml"
  echo "$dir"
}

# edit <file> <sed script> — edit a fixture file in place, the same way on GNU and BSD sed.
edit() {
  sed "$2" "$1" > "$1.tmp" && mv "$1.tmp" "$1"
}

expect() {
  local want="$1" name="$2" dir="$3" needle="${4:-}"
  local out rc
  out="$(bash "$CHECK" "$dir" 2>&1)"; rc=$?
  if [[ "$want" == "pass" && $rc -eq 0 ]] \
     || [[ "$want" == "fail" && $rc -ne 0 && ( -z "$needle" || "$out" == *"$needle"* ) ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected $want${needle:+ with needle \"$needle\"}, exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

d="$(fixture baseline)"
expect pass "the Dockerfile, WebKitGTK, both Features, and a pinned channel pass" "$d"

d="$(fixture image)"
cat > "$d/.devcontainer/devcontainer.json" <<'JSON'
{
  "name": "fixture",
  "image": "mcr.microsoft.com/devcontainers/base:debian",
  "features": {
    "ghcr.io/devcontainers/features/rust:1": {},
    "ghcr.io/devcontainers-extra/features/bun:1": { "version": "1.4.1" }
  }
}
JSON
expect fail "an image instead of the Dockerfile" "$d" "does not build from the Dockerfile"

d="$(fixture commented-dockerfile)"
edit "$d/.devcontainer/devcontainer.json" 's|    "dockerfile": "Dockerfile"|    // "dockerfile": "Dockerfile"|'
expect fail "a Dockerfile named only in a comment" "$d" "does not build from the Dockerfile"

d="$(fixture no-rust)"
edit "$d/.devcontainer/devcontainer.json" '/features\/rust:1/d'
expect fail "the Rust Feature missing" "$d" "lacks the Rust Feature"

d="$(fixture no-bun)"
edit "$d/.devcontainer/devcontainer.json" '/features\/bun:1/d'
expect fail "the Bun Feature missing" "$d" "lacks the Bun Feature"

d="$(fixture other-base)"
edit "$d/.devcontainer/Dockerfile" 's|^FROM .*|FROM ubuntu:24.04|'
expect fail "another base image" "$d" "does not start from"

d="$(fixture no-webkit)"
edit "$d/.devcontainer/Dockerfile" 's| libwebkit2gtk-4.1-dev| curl|'
expect fail "WebKitGTK not installed" "$d" "does not install libwebkit2gtk-4.1-dev"

d="$(fixture no-toolchain)"
rm "$d/rust-toolchain.toml"
expect fail "no rust-toolchain.toml" "$d" "pins no channel"

d="$(fixture repository)"
expect pass "this repository passes" "$ROOT"

if ((failed)); then
  echo "Dev Container toolchain self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Dev Container toolchain self-test passed ($passed cases)."
