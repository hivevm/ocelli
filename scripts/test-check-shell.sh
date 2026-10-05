#!/usr/bin/env bash
#
# Self-test of scripts/check-shell.sh.
#
# Each case builds a throwaway git repository with one shell script that ShellCheck flags, runs
# the lint against it, and asserts the exit code: a tracked script and a new one git does not
# ignore fail, a script under an ignored directory and a git hook in .githooks/ are read as git
# lists them, and a directory that is no repository fails rather than passing empty. GNU sed's
# in-place flag fails and names the line, while a temporary file, a script marked Linux-only, and
# a comment pass. Where
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

# GNU sed's in-place flag, spelled without the literal text so this file passes its own lint.
IN_PLACE="$(printf '%s %s' sed -i)"

d="$(repo sed-in-place)"
# shellcheck disable=SC2016  # the fixture's "$1" is its own
printf '#!/usr/bin/env bash\n%s %s "$1"\n' "$IN_PLACE" "'s/a/b/'" > "$d/edit.sh"
git -C "${d:?}" add edit.sh
expect 1 "a script that edits in place with sed fails" "$d"
out="$(bash "$CHECK" "$d" 2>&1)"
if [[ "$out" == *"edit.sh:2:"*"temporary file"* ]]; then
  passed=$((passed + 1))
else
  failed=$((failed + 1))
  echo "FAIL: the finding names the file, the line, and a portable spelling" >&2
  printf '    %s\n' "$out" >&2
fi

d="$(repo sed-temporary-file)"
# shellcheck disable=SC2016  # the fixture's "$1" is its own
printf '#!/usr/bin/env bash\nsed %s "$1" > "$1.tmp" && mv "$1.tmp" "$1"\n' "'s/a/b/'" > "$d/edit.sh"
git -C "${d:?}" add edit.sh
expect 0 "a script that writes through a temporary file passes" "$d"

d="$(repo sed-linux-only)"
# shellcheck disable=SC2016  # the fixture's "$1" is its own
printf '#!/usr/bin/env bash\n# portability: linux-only\n%s %s "$1"\n' "$IN_PLACE" "'s/a/b/'" > "$d/edit.sh"
git -C "${d:?}" add edit.sh
expect 0 "a script marked Linux-only may edit in place" "$d"

d="$(repo sed-comment)"
printf '#!/usr/bin/env bash\n# Never %s here.\necho ok\n' "$IN_PLACE" > "$d/edit.sh"
git -C "${d:?}" add edit.sh
expect 0 "a comment naming the flag passes" "$d"

if ((failed)); then
  echo "Shell lint self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Shell lint self-test passed ($passed cases)."
exit 0
