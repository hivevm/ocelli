#!/usr/bin/env bash
#
# Self-test of scripts/check-shell.sh.
#
# Each case builds a throwaway git repository with one shell script that ShellCheck flags, runs
# the lint against it, and asserts the exit code: a tracked script and a new one git does not
# ignore fail, a script under an ignored directory and a git hook in .githooks/ are read as git
# lists them, and a directory that is no repository fails rather than passing empty. Where
# ShellCheck is not on PATH the lint skips itself, and so does this test.
#
# Usage:
#     scripts/test-check-shell.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check-shell.sh"

if ! command -v shellcheck >/dev/null 2>&1; then
  echo "Shell lint self-test: shellcheck is not installed — skipped (CI runs it)."
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failed=0
passed=0

# A script ShellCheck flags: an unquoted expansion (SC2086).
FLAGGED=$'#!/usr/bin/env bash\necho $1\n'

# repo <name> — an empty fixture repository with a .gitignore for build/, printed as its path.
repo() {
  local dir="$work/$1"
  mkdir -p "$dir"
  git -C "${dir:?}" init -q -b main
  echo "build/" > "$dir/.gitignore"
  echo "$dir"
}

# expect <exit code> <name> <dir> — run the lint against the fixture.
expect() {
  local want="$1" name="$2" dir="$3" out rc
  out="$(bash "$CHECK" "${dir:?}" 2>&1)"; rc=$?
  if ((rc == want)); then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected exit $want, got exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

d="$(repo tracked)"
printf '%s' "$FLAGGED" > "$d/run.sh"
git -C "${d:?}" add run.sh
expect 1 "a tracked script with a finding fails" "$d"

d="$(repo untracked)"
printf '%s' "$FLAGGED" > "$d/new.sh"
expect 1 "a new script git does not ignore fails" "$d"

d="$(repo ignored)"
mkdir -p "$d/build"
printf '%s' "$FLAGGED" > "$d/build/generated.sh"
expect 0 "a script under an ignored directory is no part of the gate" "$d"

d="$work/no-repository"
mkdir -p "$d"
printf '%s' "$FLAGGED" > "$d/run.sh"
expect 1 "a directory git does not know fails rather than passing with nothing to lint" "$d"

d="$(repo hook)"
mkdir -p "$d/.githooks"
printf '%s' "$FLAGGED" > "$d/.githooks/pre-commit"
expect 1 "a git hook without an extension is linted" "$d"

if ((failed)); then
  echo "Shell lint self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Shell lint self-test passed ($passed cases)."
exit 0
