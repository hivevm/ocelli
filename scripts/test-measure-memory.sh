#!/usr/bin/env bash
#
# Self-test of scripts/measure-memory.sh.
#
# A stand-in for the desktop application reports the commits of the repository it is given, as
# the application reports its loaded graph, starts a child process, and keeps running; the script
# measures it on a small generated repository without a display. The stand-in's memory, its child
# included, holds a generous limit and misses a limit of 0 MB, which is reported without failing
# the run. A stand-in that exits at once, one that never reports a loaded graph, and one whose two
# runs differ by far more than 10 percent each fail the run instead of giving a verdict. The
# verdict alone is tried at its boundaries: one kB below the limit, at it, and at a spread of
# exactly 10 percent.
#
# The script it tests reads /proc and is Linux-only by design (#11), so off Linux the self-test
# skips with a notice, as the shell lint does without ShellCheck, and CI runs it on Linux.
#
# Usage:
#     scripts/test-measure-memory.sh
# Exit code 0 when every case passes or the host is not Linux, 1 otherwise.

set -uo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  echo "Memory measurement self-test: $(uname -s) is not Linux, which the measurement needs — skipped (CI runs it on Linux)."
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/measure-memory.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The stand-ins' own "$1" and "$(...)" are theirs, not this script's.
# shellcheck disable=SC2016
{
  printf '#!/usr/bin/env bash\n[[ -d "$1" ]] || exit 3\n'
  printf 'echo "$(git -C "$1" rev-list --count --all) commits" >&2\nsleep 301 &\nwait\n'
} > "$work/runs"
printf '#!/usr/bin/env bash\nexit 0\n' > "$work/exits"
printf '#!/usr/bin/env bash\nsleep 301 &\nwait\n' > "$work/silent"
# shellcheck disable=SC2016
{
  printf '#!/usr/bin/env bash\necho "1 commits" >&2\n'
  printf 'if [[ -e "%s/ran" ]]; then big=$(head -c 30000000 /dev/zero | tr "\\\\0" a); fi\n' "$work"
  printf 'touch "%s/ran"\nsleep 301 &\nwait\n' "$work"
} > "$work/grows"
chmod +x "$work/runs" "$work/exits" "$work/silent" "$work/grows"

export OCELLI_MEMORY_COMMITS=120 OCELLI_MEMORY_WRAPPER="" OCELLI_MEMORY_SETTLE=3

failed=0
passed=0

expect() {
  local name="$1" want_rc="$2" needle="$3"
  shift 3
  local out rc
  out="$(env "$@" bash "$SCRIPT" 2>&1)"; rc=$?
  if [[ $rc -eq $want_rc && "$out" == *"$needle"* ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected exit $want_rc with \"$needle\", exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

expect "a generated repository of 120 commits, all laid out, holds a generous limit" 0 \
  "| 120 | 120 |" OCELLI_DESKTOP="$work/runs" OCELLI_MEMORY_LIMIT_MB=1000
expect "a limit below the application's memory is missed and only reported" 0 \
  "| 0 MB | missed |" OCELLI_DESKTOP="$work/runs" OCELLI_MEMORY_LIMIT_MB=0
expect "an application that exits fails the run" 1 "the application exited" \
  OCELLI_DESKTOP="$work/exits"
expect "an application that never reports a loaded graph fails the run" 1 \
  "never reported a loaded graph" OCELLI_DESKTOP="$work/silent" OCELLI_MEMORY_COMMITS=10 \
  OCELLI_MEMORY_TIMEOUT=4
expect "two runs far apart give no verdict and fail the run" 1 "no verdict" \
  OCELLI_DESKTOP="$work/grows" OCELLI_MEMORY_LIMIT_MB=1000

# verdict <name> <first kB> <second kB> <exit> <verdict> — the verdict alone, at its boundaries,
# against the limit of 200 MB.
verdict() {
  local out rc
  out="$(OCELLI_MEMORY_LIMIT_MB=200 bash "$SCRIPT" --verdict "$2" "$3")"; rc=$?
  if [[ $rc -eq $4 && "$out" == "$5"* ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $1 (expected exit $4 with \"$5\", exit $rc: $out)" >&2
  fi
}
verdict "one kB below the limit holds Q-3" 204799 204700 0 "holds"
verdict "exactly the limit misses Q-3" 204800 204700 0 "missed"
verdict "runs exactly 10 percent apart give no verdict" 1000 900 1 "no verdict"
verdict "runs just under 10 percent apart give a verdict" 1000 901 0 "holds"

if pgrep -f '^sleep 301$' >/dev/null; then
  failed=$((failed + 1))
  echo "FAIL: a stand-in's child process was left running" >&2
else
  passed=$((passed + 1))
fi

if ((failed)); then
  echo "Memory measurement self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Memory measurement self-test passed ($passed cases)."
