#!/usr/bin/env bash
#
# Shell lint for this repository.
#
# Runs ShellCheck over every shell script of the repository: every '*.sh' file git does not
# ignore, and the git hooks under .githooks/, which git requires to be named without an extension.
# The files come from git, as in every other check, so a script under .scratch/ or under a build
# directory the project ignores is no part of the gate. The scripts under scripts/ and the hooks
# are the repository's entire enforcement layer, so they are held to a lint gate themselves.
#
# ShellCheck is the one check dependency beyond bash + coreutils/git. CI does not install it — the
# GitHub-hosted runner image ships it preinstalled, and the Dev Container installs it on creation
# (.devcontainer/devcontainer.json). Elsewhere, when shellcheck is not on PATH, the
# check skips itself with a notice instead of failing (CI remains the enforcing gate); in a CI
# run (CI=true) a missing shellcheck is an error, never a silent skip.
#
# Usage:
#     scripts/check-shell.sh [repository root]        (or: bash scripts/check-shell.sh)
# The root defaults to this repository; scripts/test-check-shell.sh passes fixtures.
# Exit code 0 when all scripts pass (or shellcheck is unavailable outside CI), 1 otherwise, 2 on a
# root that is no directory.

set -uo pipefail

ROOT="$(cd "${1:-$(dirname "${BASH_SOURCE[0]}")/..}" 2>/dev/null && pwd)" \
  || { echo "Shell lint: '${1:-}' is not a directory." >&2; exit 2; }

if ! command -v shellcheck >/dev/null 2>&1; then
  if [[ "${CI:-}" == "true" ]]; then
    echo "ERROR: shellcheck not found in a CI run — the runner image is expected to ship it." >&2
    exit 1
  fi
  echo "shellcheck is not installed — skipped (CI enforces this check)."
  echo "Install it to run locally: https://www.shellcheck.net"
  exit 0
fi

# A list git cannot give is an error, not an empty gate.
if ! listed="$(git -C "$ROOT" ls-files --cached --others --exclude-standard -- '*.sh' '.githooks/*')"; then
  echo "Shell lint: $ROOT is not a git repository — git lists the files the lint reads." >&2
  exit 1
fi
scripts=()
while IFS= read -r f; do
  [[ -n "$f" && -f "$ROOT/$f" ]] && scripts+=("$ROOT/$f")
done < <(sort -u <<< "$listed")

if ((${#scripts[@]} == 0)); then
  echo "No shell scripts found."
  exit 0
fi

if ! shellcheck "${scripts[@]}"; then
  echo
  echo "Shell lint FAILED (see findings above)."
  exit 1
fi

echo "Shell lint passed (${#scripts[@]} scripts)."
exit 0
