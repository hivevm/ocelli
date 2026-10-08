#!/usr/bin/env bash
#
# Self-test of scripts/measure-memory.sh.
#
# A stand-in for the desktop application reports the commits of the repository it is given, as
# the application reports its loaded graph, holds about 20 MB more with a repository than
# without, starts a child process, and keeps running; the script
# measures it, with no repository and on a small generated one, without a display. What the
# stand-in adds, its child included, holds a generous limit and misses a limit of 10 MB, below
# the 20 MB it adds, which fails the run. A stand-in that exits at once, one that never reports a loaded graph, and one whose two
# runs differ by far more than 10 percent each fail the run instead of giving a verdict. The
# verdict alone is tried at its boundaries: one kB less added than the limit, exactly the limit,
# which of the runs make the added memory, and a spread of exactly 10 percent in either pair.
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
# The stand-ins' child sleeps for a time this run alone uses, so that the check for one left
# running sees this run's and no other session's.
pause="301.${$}"

# The stand-ins' own "$1" and "$(...)" are theirs, not this script's.
# shellcheck disable=SC2016
{
  printf '#!/usr/bin/env bash\nif [[ -n "${1:-}" ]]; then\n  [[ -d "$1" ]] || exit 3\n'
  printf '  big=$(head -c 20000000 /dev/zero | tr "\\0" a)\n'
  printf '  echo "$(git -C "$1" rev-list --count --all) commits" >&2\nfi\nsleep %s &\nwait\n' "$pause"
} > "$work/runs"
printf '#!/usr/bin/env bash\nexit 0\n' > "$work/exits"
printf '#!/usr/bin/env bash\nsleep %s &\nwait\n' "$pause" > "$work/silent"
# shellcheck disable=SC2016
{
  printf '#!/usr/bin/env bash\necho "1 commits" >&2\n'
  printf 'if [[ -e "%s/ran" ]]; then big=$(head -c 30000000 /dev/zero | tr "\\\\0" a); fi\n' "$work"
  printf 'touch "%s/ran"\nsleep %s &\nwait\n' "$work" "$pause"
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
expect "less than the stand-in adds is missed and fails the run" 1 \
  "| 10 MB | missed |" OCELLI_DESKTOP="$work/runs" OCELLI_MEMORY_LIMIT_MB=10
expect "an application that exits fails the run" 1 "the application exited" \
  OCELLI_DESKTOP="$work/exits"
# The time limit holds the empty runs too, which must settle within it on a busy machine first.
expect "an application that never reports a loaded graph fails the run" 1 \
  "never reported a loaded graph" OCELLI_DESKTOP="$work/silent" OCELLI_MEMORY_COMMITS=10 \
  OCELLI_MEMORY_TIMEOUT=12
expect "two runs far apart give no verdict and fail the run" 1 "no verdict" \
  OCELLI_DESKTOP="$work/grows" OCELLI_MEMORY_LIMIT_MB=1000

# verdict <name> <empty> <empty> <loaded> <loaded> <exit> <verdict> — the verdict alone, in kB,
# at its boundaries, against a limit of 150 MB added (153600 kB).
verdict() {
  local out rc
  out="$(OCELLI_MEMORY_LIMIT_MB=150 bash "$SCRIPT" --verdict "$2" "$3" "$4" "$5")"; rc=$?
  if [[ $rc -eq $6 && "$out" == "$7"* ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $1 (expected exit $6 with \"$7\", exit $rc: $out)" >&2
  fi
}
verdict "one kB less added than the limit holds Q-3" 400000 400100 553599 553500 0 "holds"
verdict "exactly the limit added misses Q-3 and fails the run" 400000 400100 553600 553500 1 "missed"
verdict "the smaller empty and the larger loaded run make the added memory" \
  400100 400000 553599 553500 0 "holds"
verdict "empty runs exactly 10 percent apart give no verdict" 1000 900 1050 1050 1 "no verdict"
verdict "loaded runs exactly 10 percent apart give no verdict" 1000 1000 2000 1800 1 "no verdict"
verdict "runs just under 10 percent apart give a verdict" 1000 901 1050 1050 0 "holds"

# Another session's stand-in, running beside this one, is none of this run's.
sleep 301 &
foreign=$!
if pgrep -f "^sleep ${pause}\$" >/dev/null; then
  failed=$((failed + 1))
  echo "FAIL: a stand-in's child process was left running" >&2
else
  passed=$((passed + 1))
fi
kill "$foreign" 2>/dev/null
wait "$foreign" 2>/dev/null

if ((failed)); then
  echo "Memory measurement self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Memory measurement self-test passed ($passed cases)."
