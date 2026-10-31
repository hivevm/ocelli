#!/usr/bin/env bash
#
# Self-test of scripts/measure-windows.sh.
#
# Measures a generated repository of a few hundred commits with ocelli-measure and asserts the
# table: the commits loaded, the times, and a verdict that holds under generous limits. With
# stand-ins for ocelli-measure, a measurement that cannot be read fails the run, and so does a
# chunk taken in while loading that waits as long as the window limit. The verdict alone
# is tried at its boundaries: just under each limit holds, exactly at either limit misses and fails
# the run.
#
# Usage:
#     scripts/test-measure-windows.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/measure-windows.sh"
export OCELLI_MEASURE_ALL=1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cargo build --locked --quiet --manifest-path "$ROOT/Cargo.toml" -p ocelli-measure || exit 1
export OCELLI_MEASURE="$ROOT/target/debug/ocelli-measure"

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

expect "a generated repository of 300 commits is measured and holds generous limits" 0 \
  "| 300 |" OCELLI_WINDOWS_COMMITS=300 OCELLI_FIRST_LIMIT_MS=60000 OCELLI_WINDOW_LIMIT_MS=60000
expect "the table ends with the verdict" 0 "| holds |" \
  OCELLI_WINDOWS_COMMITS=300 OCELLI_FIRST_LIMIT_MS=60000 OCELLI_WINDOW_LIMIT_MS=60000

printf '#!/usr/bin/env bash\necho "commits: 300"\n' > "$work/stub"
# shellcheck disable=SC2016
printf '#!/usr/bin/env bash\nprintf "commits: 9\\nfirst window ms: 1.0\\nload ms: 2.0\\nslowest while loading ms: 100.0\\nwindows: 1\\nslowest window ms: 1.0\\nlineage ms: 0.1\\nsearch ms: 0.1\\n"\n' > "$work/slow"
chmod +x "$work/slow"
chmod +x "$work/stub"
expect "a measurement that cannot be read fails the run" 1 "cannot read the measurement" \
  OCELLI_WINDOWS_COMMITS=300 OCELLI_MEASURE="$work/stub"
expect "a chunk taken in at the window limit misses Q-2" 1 "| 100.0 ms | 1.0 ms | 100 ms |" \
  OCELLI_WINDOWS_COMMITS=300 OCELLI_MEASURE="$work/slow"

# verdict <name> <first ms> <slowest ms> <exit> <verdict> — against 2000 ms and 100 ms.
verdict() {
  local out rc
  out="$(OCELLI_FIRST_LIMIT_MS=2000 OCELLI_WINDOW_LIMIT_MS=100 bash "$SCRIPT" --verdict "$2" "$3")"
  rc=$?
  if [[ $rc -eq $4 && "$out" == "$5" ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $1 (expected exit $4 with \"$5\", exit $rc: $out)" >&2
  fi
}
verdict "just under both limits holds Q-2" 1999.9 99.9 0 holds
verdict "a first window of exactly the limit misses Q-2" 2000 1.0 1 missed
verdict "a window of exactly the limit misses Q-2" 10.0 100 1 missed
verdict "a first window far over the limit misses Q-2" 3378.8 1.1 1 missed

# Without OCELLI_MEASURE_ALL the measurement skips itself, as in scripts/check-all.sh, and
# measures nothing.
out="$(env -u OCELLI_MEASURE_ALL OCELLI_MEASURE=/nonexistent bash "$SCRIPT" 2>&1)"; rc=$?
if ((rc == 0)) && [[ "$out" == "Waits on a large history (Q-2): skipped"* ]]; then
  passed=$((passed + 1))
else
  failed=$((failed + 1)); echo "FAIL: without OCELLI_MEASURE_ALL the measurement skips itself (exit $rc)" >&2
fi

if ((failed)); then
  echo "Window measurement self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Window measurement self-test passed ($passed cases)."
