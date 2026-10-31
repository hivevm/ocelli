#!/usr/bin/env bash
#
# Measures Q-2 (feature spec #27): how long a developer waits on a generated repository of a
# million commits, thirty branch lines side by side and some dates running backwards (the wide
# shape of scripts/generate-history.sh). Verifies: Q-2, ADR-0016
#
# ocelli-measure loads the whole history as the front ends do, chunk by chunk through the
# repository port into the core's History, and reports the time from opening the repository until
# the first window of rows is read, the slowest chunk taken in while the rest loads, with a
# selection's lineage grown and the window in view read again as a front end does, and the slowest
# window read across the loaded history. Q-2 holds when the first window takes less than the
# first-window limit and neither a chunk taken in nor a window takes the window limit or more; otherwise the run fails, as it does when the repository cannot be
# generated or measured. The time of the whole load, of the newest commit's lineage, and of a
# search through the whole history are reported beside them and decide nothing. The script reports
# a table, and appends it to the job summary when GITHUB_STEP_SUMMARY is set.
#
# Usage:
#     scripts/measure-windows.sh
#     scripts/measure-windows.sh --verdict FIRST_MS SLOWEST_MS   (the verdict only)
# Environment, all optional: OCELLI_MEASURE, the binary (built here in release when unset);
# OCELLI_WINDOWS_COMMITS, the size of the repository (1000000); OCELLI_FIRST_LIMIT_MS, the limit
# of the first window (2000); OCELLI_WINDOW_LIMIT_MS, the limit of every window (100).
# Exit code 0 when Q-2 holds, 1 when it is missed or cannot be measured.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# A measurement takes minutes, and scripts/check-all.sh runs every script a workflow runs, so it
# skips itself unless OCELLI_MEASURE_ALL=1, which .github/workflows/measure.yml sets; the verdict
# alone always runs.
if [[ "${OCELLI_MEASURE_ALL:-}" != 1 && "${1:-}" != --verdict ]]; then
  echo "Waits on a large history (Q-2): skipped — measured weekly by .github/workflows/measure.yml; OCELLI_MEASURE_ALL=1 measures here."
  exit 0
fi
commits="${OCELLI_WINDOWS_COMMITS:-1000000}"
first_limit="${OCELLI_FIRST_LIMIT_MS:-2000}"
window_limit="${OCELLI_WINDOW_LIMIT_MS:-100}"

# judge <first ms> <slowest ms> — set verdict and rc: Q-2 holds when the first window stays below
# its limit and the slowest window below its own. The times may carry a fraction.
judge() {
  if awk -v f="$1" -v w="$2" -v fl="$first_limit" -v wl="$window_limit" \
    'BEGIN { exit !(f < fl && w < wl) }'; then
    verdict="holds"
    rc=0
  else
    verdict="missed"
    rc=1
  fi
}

if [[ "${1:-}" == --verdict ]]; then
  judge "$2" "$3"
  echo "$verdict"
  exit "$rc"
fi

# indent — the lines of standard input, indented under a message.
indent() { while IFS= read -r l; do printf '    %s\n' "$l"; done; }

measure="${OCELLI_MEASURE:-}"
if [[ -z "$measure" ]]; then
  cargo build --locked --quiet --release --manifest-path "$ROOT/Cargo.toml" -p ocelli-measure \
    || { echo "windows: cannot build ocelli-measure" >&2; exit 1; }
  measure="$ROOT/target/release/ocelli-measure"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

bash "$ROOT/scripts/generate-history.sh" "$work/repo.git" "$commits" wide \
  || { echo "windows: cannot generate the repository" >&2; exit 1; }
out="$("$measure" windows "$work/repo.git" 2>&1)" \
  || { echo "windows: cannot measure the repository:" >&2; indent <<< "$out" >&2; exit 1; }

# value <key> — the number after "<key>: " in the measurement.
value() { sed -n "s/^$1: \([0-9.]*\)$/\1/p" <<< "$out"; }
loaded="$(value commits)" first="$(value 'first window ms')" slowest="$(value 'slowest window ms')"
loading="$(value 'slowest while loading ms')"
if [[ -z "$loaded" || -z "$first" || -z "$slowest" || -z "$loading" ]]; then
  echo "windows: cannot read the measurement:" >&2
  indent <<< "$out" >&2
  exit 1
fi
judge "$first" "$(awk -v a="$slowest" -v b="$loading" 'BEGIN { print (a > b ? a : b) }')"

report="| Commits | First window | Limit | Slowest chunk taken in | Slowest of $(value windows) windows | Limit | Load | Lineage | Search | Q-2 |
|---|---|---|---|---|---|---|---|---|---|"
report+=$'\n'"| $loaded | $first ms | $first_limit ms | $loading ms | $slowest ms | $window_limit ms | $(value 'load ms') ms | $(value 'lineage ms') ms | $(value 'search ms') ms | $verdict |"

echo "$report"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  { echo "## Q-2: waiting on a history of $commits commits"; echo; echo "$report"; } \
    >> "$GITHUB_STEP_SUMMARY"
fi
exit "$rc"
