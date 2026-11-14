#!/usr/bin/env bash
#
# Self-test of scripts/check-adr-text.sh.
#
# Each case builds a throwaway git repository under a temporary directory whose main, tagged
# 'base', holds an accepted, a superseded, and a proposed ADR in the form docs/adr/template.md
# writes, commits on a branch at it what an agent or a human would, and runs the check against the
# base, asserting the exit code and the line a reader would meet. A sentence changed, added, or
# removed in an accepted ADR, a line that begins like a diff's own, a header field rewritten or
# written into the body, a decider added, an accepted ADR deleted or renamed, a superseded one
# changed, one accepted on the branch and then edited, an edit made while resolving a merge, a
# merge whose resolution deletes an accepted ADR, an octopus merge that deletes one, a path with a
# space, and a commit that names a normalization only in its text fail. The Status line flipped
# to superseded, the Date line, a proposed ADR, a NUC- ADR of the base, a merge from main, a commit
# carrying the normalization's trailer, a branch's normalization merged into main, the template
# repository, and a base of zeros, with main and without, pass.
#
# Verifies: NUC-decisions-are-proposed-then-accepted
# Inputs: scripts/check-adr-text.sh scripts/lib/
#
# Usage:
#     scripts/test-check-adr-text.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

# shellcheck source-path=SCRIPTDIR source=lib/selftest.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/selftest.sh"
CHECK="$ROOT/scripts/check-adr-text.sh"

# g <dir> <git args...> — git in the fixture with a fixed identity.
g() {
  fixture "$1"
  local dir="$1"
  shift
  git -C "${dir:?}" -c user.name=fixture -c user.email=fixture@example.invalid "$@"
}

# adr_id <n> — the identifier TST-decision-000n of the nth fixture ADR. TST is a category no index
# defines, so the documentation checks read no citation in this file.
adr_id() { printf "TST-decision-%04d" "$1"; }

# adr <dir> <number> <status> — an ADR in the form of docs/adr/template.md.
adr() {
  mkdir -p "$1/docs/adr"
  cat > "$1/docs/adr/TST-decision-$2.md" <<ADR
# TST-decision-$2: Decision $2

- **Status:** $3
- **Date:** 2026-11-01
- **Deciders:** Markus Brigl
- **Applies to:** everything

## Context

The context of decision $2.

## Decision

The decision $2 takes.
ADR
}

# repo <name> — a repository whose main, tagged 'base', holds the first ADR accepted, the second
# superseded, and the third proposed, with the branch work at it checked out, printed as its path.
repo() {
  local dir="$work/$1"
  mkdir -p "$dir"
  git -C "${dir:?}" init -q -b main
  adr "$dir" 0001 "🟢 accepted"
  adr "$dir" 0002 "⚪ superseded by [$(adr_id 1)](TST-decision-0001.md)"
  adr "$dir" 0003 "🟡 proposed"
  g "$dir" add -A
  g "$dir" commit -q -m "initial"
  g "$dir" tag base
  g "$dir" switch -q -c work
  echo "$dir"
}

# edit <dir> <path> <sed script> <message...> — edit a file in place, the same way on GNU and BSD
# sed, and commit it with the messages, one paragraph each.
edit() {
  local dir="$1" path="$2" script="$3" m
  shift 3
  sed "$script" "$dir/$path" > "$dir/$path.tmp" && mv "$dir/$path.tmp" "$dir/$path"
  local args=()
  for m in "$@"; do args+=(-m "$m"); done
  g "$dir" add -A
  g "$dir" commit -q "${args[@]}"
}

# expect <exit code> <name> <dir> <needle> [base] — run the check against the given base (default
# 'base'); the output must contain the needle.
expect() {
  local want="$1" name="$2" dir="$3" needle="$4" base="${5-base}"
  local out rc
  fixture "$dir"
  out="$(bash "$CHECK" --root "$dir" ${base:+"$base"} 2>&1)"; rc=$?
  if [[ $rc -eq "$want" && "$out" == *"$needle"* ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected exit $want with \"$needle\", got exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

A=docs/adr/TST-decision-0001.md

d="$(repo sentence)"
edit "$d" "$A" 's/The decision 0001 takes./The decision 0001 takes, and more./' "Reword"
expect 1 "a sentence of an accepted ADR changed" "$d" \
  "changes 2 lines of $(adr_id 1) ($A), which is accepted, the first at line 14: -The decision 0001 takes."

d="$(repo added)"
echo "A sentence added." >> "$d/$A"
g "$d" commit -q -a -m "Add"
expect 1 "a sentence added to an accepted ADR" "$d" "changes 1 line of $(adr_id 1)"

d="$(repo applies)"
edit "$d" "$A" 's/^- \*\*Applies to:\*\* everything/- **Applies to:** something/' "Narrow"
expect 1 "another header line of an accepted ADR" "$d" "the first at line 6: -- **Applies to:** everything"

d="$(repo superseded)"
edit "$d" docs/adr/TST-decision-0002.md 's/The decision 0002 takes./Taken back./' "Reword"
expect 1 "a superseded ADR, accepted before" "$d" "changes 2 lines of $(adr_id 2)"

d="$(repo deleted)"
g "$d" rm -q "$A"
g "$d" commit -q -m "Drop"
expect 1 "an accepted ADR deleted" "$d" "deletes $(adr_id 1) ($A), which is accepted"

d="$(repo renamed)"
g "$d" mv "$A" docs/adr/TST-decision-0004.md
g "$d" commit -q -m "Rename"
expect 1 "an accepted ADR renamed outside a normalization" "$d" "deletes $(adr_id 1)"

d="$(repo later)"
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes now/' "Fine"
edit "$d" "$A" 's/0001 takes/0001 takes now/' "Second"
echo "notes" > "$d/NOTES.md"
g "$d" add -A
g "$d" commit -q -m "Third"
expect 1 "the commit that changes it named among others" "$d" "$(g "$d" rev-parse --short HEAD~1) changes"

d="$(repo status)"
edit "$d" docs/adr/TST-decision-0003.md 's/🟡 proposed/🟢 accepted/; s/2026-11-01/2026-11-04/' "Accept"
edit "$d" "$A" "s/🟢 accepted/⚪ superseded by [$(adr_id 3)](TST-decision-0003.md)/; s/2026-11-01/2026-11-04/" \
  "Supersede"
expect 0 "the Status and Date lines flipped" "$d" "no accepted ADR's text changed"

d="$(repo deciders)"
edit "$d" "$A" 's/^- \*\*Deciders:\*\* Markus Brigl/- **Deciders:** Markus Brigl, the project/' "Adopt"
expect 1 "a decider added, which no rule permits since the inherited ADRs went" "$d" \
  "the first at line 5: -- **Deciders:** Markus Brigl"

d="$(repo proposed)"
edit "$d" docs/adr/TST-decision-0003.md 's/The decision 0003 takes./Another decision./' "Revise"
expect 0 "the text of a proposed ADR" "$d" "no accepted ADR's text changed"

d="$(repo normalization)"
g "$d" mv "$A" docs/adr/TST-decision-0004.md
edit "$d" docs/adr/TST-decision-0004.md "s/$(adr_id 1)/$(adr_id 4)/" "docs: normalize the record" \
  "Map: $(adr_id 1) -> $(adr_id 4)" "Normalization: 2026-11-04"
expect 0 "a normalization's commit with its trailer" "$d" "no accepted ADR's text changed"

d="$(repo prose)"
edit "$d" "$A" 's/0001 takes/0001 takes now/' "docs: Normalization: 2026-11-04 in the subject" \
  "Normalization: 2026-11-04" "A paragraph after it, so it is no trailer."
expect 1 "a normalization named in the text, no trailer" "$d" "changes 2 lines of $(adr_id 1)"

d="$(repo flipped-then-edited)"
edit "$d" docs/adr/TST-decision-0003.md 's/🟡 proposed/🟢 accepted/' "Accept"
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes now/' "Then reword"
expect 1 "an ADR accepted on the branch, then edited" "$d" "changes 2 lines of $(adr_id 3)"

d="$(repo status-reworded)"
edit "$d" "$A" 's/🟢 accepted/🟢 accepted, except where it is not/' "Qualify"
expect 1 "the Status line reworded, not flipped" "$d" "the first at line 3: +- **Status:**"

d="$(repo deciders-rewritten)"
edit "$d" "$A" 's/^- \*\*Deciders:\*\* Markus Brigl/- **Deciders:** someone else/' "Replace"
expect 1 "the Deciders line rewritten" "$d" "changes 2 lines of $(adr_id 1)"

d="$(repo field-in-body)"
echo "- **Status:** a list item in the body" >> "$d/$A"
g "$d" commit -q -a -m "Body"
expect 1 "a header field written into the body" "$d" "changes 1 line of $(adr_id 1)"

d="$(repo dash-line)"
edit "$d" "$A" 's/^The context of decision 0001./-- the context/' "Dash"
edit "$d" "$A" '/^-- the context/d' "Remove"
expect 1 "a line that begins like a diff's own header" "$d" "$(g "$d" rev-parse --short HEAD~1) changes 2 lines"

d="$(repo spaced)"
adr "$d" 0004 "🟢 accepted"
mv "$d/docs/adr/TST-decision-0004.md" "$d/docs/adr/TST-a decision.md"
g "$d" add -A
g "$d" commit -q -m "Accept 0004"
edit "$d" "docs/adr/TST-a decision.md" 's/0004 takes/0004 takes now/' "Reword"
expect 1 "an accepted ADR whose file name holds a space" "$d" "TST-a decision (docs/adr/TST-a decision.md)"

d="$(repo merged)"
g "$d" switch -q main
edit "$d" docs/adr/TST-decision-0003.md 's/🟡 proposed/🟢 accepted/' "Accept 0003 on main"
g "$d" switch -q work
edit "$d" docs/adr/TST-decision-0003.md 's/^The context of decision 0003./The context, revised./' "Revise"
g "$d" merge -q --no-edit main > /dev/null
expect 0 "a merge from main that accepts an ADR the branch revised before" "$d" \
  "no accepted ADR's text changed"
g "$d" switch -q main
edit "$d" "$A" 's/^The context of decision 0001./The context of 0001, on main./' "Main reword"
g "$d" switch -q work
edit "$d" "$A" 's/^The context of decision 0001./The context of 0001, on the branch./' "Branch reword"
g "$d" merge -q main > /dev/null 2>&1
# Resolved by keeping neither side's line: an edit of the resolution's own.
grep -v -e '^<<<<<<<' -e '^=======' -e '^>>>>>>>' -e '^The context of 0001, on' "$d/$A" > "$d/$A.tmp"
mv "$d/$A.tmp" "$d/$A"
g "$d" add "$A"
g "$d" commit -q --no-edit
expect 1 "an edit made while resolving a merge" "$d" "$(g "$d" rev-parse --short HEAD) changes"

d="$(repo merged-normalization)"
g "$d" switch -q main
edit "$d" "$A" 's/0001 takes/0001 takes, restated/' "docs: normalize the record" "Normalization: 2026-11-04"
g "$d" switch -q work
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes now/' "Revise"
g "$d" merge -q --no-edit main > /dev/null
g "$d" tag base-of-merge HEAD~1
expect 0 "a merge that brings main's normalization along" "$d" "no accepted ADR's text changed" \
  base-of-merge

d="$(repo normalization-merged)"
g "$d" rm -q "$A"
g "$d" commit -q -m "docs: normalize the record" -m "Normalization: 2026-11-11"
g "$d" switch -q main
g "$d" merge -q --no-ff --no-edit work > /dev/null
expect 0 "a branch's normalization merged into main" "$d" "as a normalization, by their trailer"

d="$(repo resolution-deletes)"
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes now/' "Revise"
g "$d" switch -q main
g "$d" merge -q --no-ff --no-commit work > /dev/null 2>&1
g "$d" rm -q "$A"
g "$d" commit -q --no-edit
expect 1 "a merge whose resolution deletes an accepted ADR" "$d" \
  "$(g "$d" rev-parse --short HEAD) deletes $(adr_id 1)"

d="$(repo octopus)"
echo "notes" > "$d/notes.txt"
g "$d" add notes.txt
g "$d" commit -q -m "Notes"
g "$d" switch -q -c side main
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes on the side/' "Side"
g "$d" switch -q main
g "$d" merge -q --no-ff --no-commit work side > /dev/null 2>&1
g "$d" rm -q "$A"
g "$d" commit -q -m "Octopus"
check "the octopus fixture merges three parents" test "$(g "$d" log -1 --format=%p | wc -w)" -eq 3
expect 1 "an octopus merge that deletes an accepted ADR" "$d" \
  "$(g "$d" rev-parse --short HEAD) deletes $(adr_id 1)"

d="$(repo octopus-rewrites)"
echo "notes" > "$d/notes.txt"
g "$d" add notes.txt
g "$d" commit -q -m "Notes"
g "$d" switch -q -c side main
edit "$d" docs/adr/TST-decision-0003.md 's/0003 takes/0003 takes on the side/' "Side"
g "$d" switch -q main
g "$d" merge -q --no-ff --no-commit work side > /dev/null 2>&1
edit "$d" "$A" 's/0001 takes/0001 takes now/' "Octopus"
expect 1 "an octopus merge that rewrites an accepted ADR" "$d" \
  "$(g "$d" rev-parse --short HEAD) changes 2 lines of $(adr_id 1)"

d="$(repo zeros-main)"
edit "$d" "$A" 's/0001 takes/0001 takes now/' "Reword"
expect 1 "a base of zeros with main to compare with" "$d" "changes 2 lines of $(adr_id 1)" \
  0000000000000000000000000000000000000000

# The base ADR's file, assembled so that the documentation checks read no citation of it here.
nuc="NUC"
B="docs/adr/$nuc-base-rule.md"
d="$(repo base)"
mkdir -p "$d/docs/adr"
printf '# %s-base-rule: A rule\n\n- **Status:** 🟢 accepted\n\n## Decision\n\nOne.\n' "$nuc" > "$d/$B"
g "$d" add -A
g "$d" commit -q -m "Take up the base"
g "$d" tag base-taken
edit "$d" "$B" 's/One./Two./' "Take up a later release"
expect 0 "the NUC- base, which a take-up replaces" "$d" "no accepted ADR's text changed" base-taken

d="$(repo template)"
echo "setup" > "$d/TEMPLATE-SETUP.md"
edit "$d" "$A" 's/0001 takes/0001 takes now/' "Correct"
expect 0 "the template repository" "$d" "corrects its own ADRs in place"

d="$(repo zeros)"
edit "$d" "$A" 's/0001 takes/0001 takes now/' "Reword"
g "$d" branch -q -D main
expect 0 "a base of zeros and no main to compare with" "$d" "nothing to compare" \
  0000000000000000000000000000000000000000

expect 2 "a base that does not resolve" "$d" "does not resolve" no-such-ref

summary "Accepted ADR text self-test"
