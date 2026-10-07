#!/usr/bin/env bash
#
# Self-test of scripts/measure-crossings.sh.
#
# Builds two small repositories, a straight history and one with a merged branch, measures a
# corpus naming them at fixed commits with their recorded counts, and asserts the table: one row
# per repository with the commits, Ocelli's count, the record, git's count, and a verdict. With a
# stand-in for ocelli-measure the verdict is tried at the record: below and at it Q-1 holds
# whatever git draws, one above it misses Q-1 and fails the run. A commit that does not exist, a
# repository without a record, a layout without the main line, and counts that cannot be read fail
# the run too.
#
# Usage:
#     scripts/test-measure-crossings.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/measure-crossings.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cargo build --locked --quiet --manifest-path "$ROOT/Cargo.toml" -p ocelli-measure || exit 1
export OCELLI_MEASURE="$ROOT/target/debug/ocelli-measure"

failed=0
passed=0

# url <dir> — a file URL git understands for a local directory; on Windows git reads a path like
# /tmp/... against the current drive, so the URL carries the Windows path there.
url() {
  if command -v cygpath >/dev/null 2>&1; then
    echo "file:///$(cygpath -m "$1")"
  else
    echo "file://$1"
  fi
}

# commit <repo> <message> — commit an empty change at a fixed time.
commit() {
  GIT_AUTHOR_DATE="2026-01-01T00:00:0${3:-0}Z" GIT_COMMITTER_DATE="2026-01-01T00:00:0${3:-0}Z" \
    git -C "$1" -c user.name=Fixture -c user.email=fixture@example.invalid \
    commit --quiet --allow-empty -m "$2"
}

git init --quiet -b main "$work/straight"
commit "$work/straight" one 1
commit "$work/straight" two 2
straight="$(git -C "$work/straight" rev-parse HEAD)"

git init --quiet -b main "$work/merged"
commit "$work/merged" base 1
git -C "$work/merged" switch --quiet -c topic
commit "$work/merged" topic 2
git -C "$work/merged" switch --quiet main
commit "$work/merged" main 3
GIT_COMMITTER_DATE="2026-01-01T00:00:04Z" git -C "$work/merged" -c user.name=Fixture \
  -c user.email=fixture@example.invalid merge --quiet --no-ff -m merge topic
merged="$(git -C "$work/merged" rev-parse HEAD)"

expect() {
  local name="$1" want_rc="$2" needle="$3" corpus="$4"
  local out rc
  out="$(bash "$SCRIPT" "$corpus" 2>&1)"; rc=$?
  if [[ $rc -eq $want_rc && "$out" == *"$needle"* ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected exit $want_rc with \"$needle\", exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

printf 'straight %s %s 0\nmerged %s %s 0\n' \
  "$(url "$work/straight")" "$straight" "$(url "$work/merged")" "$merged" > "$work/corpus"
expect "a straight history is measured against its record" 0 \
  "| straight | ${straight:0:7} | 2 | 0 | 0 | 0 | holds |" "$work/corpus"
expect "a merged branch is measured" 0 "| merged | ${merged:0:7} | 4 |" "$work/corpus"
expect "the run ends with Q-1 holding on every repository" 0 "Q-1 holds on each" "$work/corpus"

printf 'gone %s %s 0\n' "$(url "$work/straight")" 0000000000000000000000000000000000000001 \
  > "$work/missing"
expect "a commit that does not exist fails the run" 1 "Measurement FAILED for: gone" "$work/missing"

printf 'unrecorded %s %s\n' "$(url "$work/straight")" "$straight" > "$work/unrecorded"
expect "a repository without a recorded count fails the run" 1 \
  "Measurement FAILED for: unrecorded" "$work/unrecorded"

# A stand-in for ocelli-measure that prints chosen counts decides the verdict without a layout.
stub() {
  printf '#!/usr/bin/env bash\nprintf "%s"\n' "$1" > "$work/stub"
  chmod +x "$work/stub"
  echo "$work/stub"
}
printf 'straight %s %s 4\n' "$(url "$work/straight")" "$straight" > "$work/one"
OCELLI_MEASURE="$(stub 'commits: 9\\nmain line: abc\\nocelli crossings: 3\\ngit crossings: 1\\n')" \
  expect "fewer crossings than recorded hold Q-1, whatever git draws" 0 "| 9 | 3 | 4 | 1 | holds |" "$work/one"
OCELLI_MEASURE="$(stub 'commits: 9\\nmain line: abc\\nocelli crossings: 4\\ngit crossings: 9\\n')" \
  expect "exactly the recorded crossings hold Q-1" 0 "| 9 | 4 | 4 | 9 | holds |" "$work/one"
OCELLI_MEASURE="$(stub 'commits: 9\\nmain line: abc\\nocelli crossings: 5\\ngit crossings: 9\\n')" \
  expect "one crossing more than recorded misses Q-1 and fails the run" 1 "Q-1 MISSED on: straight" "$work/one"
OCELLI_MEASURE="$(stub 'commits: 9\\nmain line: none\\nocelli crossings: 3\\ngit crossings: 1\\n')" \
  expect "a layout without the main line fails the run" 1 "Measurement FAILED for: straight" "$work/one"
OCELLI_MEASURE="$(stub 'commits: 9\\n')" \
  expect "counts that cannot be read fail the run" 1 "Measurement FAILED for: straight" "$work/one"

if ((failed)); then
  echo "Measurement self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Measurement self-test passed ($passed cases)."
