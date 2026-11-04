#!/usr/bin/env bash
#
# Check: the text of an accepted ADR stays as it was accepted (AGENTS.md, section 3, rule 5;
# ADR-0001).
#
# Outside a normalization, an accepted ADR is changed only by superseding it, and a human makes
# the two edits the rule permits: the flip of its Status line, to superseded, and the project's
# name added to its Deciders line. Its Date line moves with the flip. Every commit between a base
# and a head is read, a merge by what its resolution changed beyond the merge git would make
# itself (--remerge-diff). One that deletes an ADR accepted or superseded before it, or changes a
# line of one other than those three lines of its header, fails the check, naming the commit,
# the ADR, and the line. Whether an ADR is held to this is read before each commit: a proposed one
# is still developed with the human and changes freely, until a commit accepts it. Since every
# commit is read, an edit a later commit undoes still fails until the history is rewritten.
#
# The two exceptions the rule names pass. A normalization rewrites and renumbers the whole record;
# each of its commits that changes such an ADR carries the trailer 'Normalization: <date>', as
# docs/adr/README.md (Normalization, References) says. Any commit can carry it, so each commit
# that passes by it is listed, for the human who reads the run. The template repository, which
# still carries TEMPLATE-SETUP.md, corrects its own ADRs in place (ADR-0005), so there the check
# passes whole.
#
# Pure bash + git (2.36 or later, for --remerge-diff). Usage:
#     scripts/check-adr-text.sh [--root <dir>] [base [head]]
# It reads the repository it lives in, as the checks do; --root names another, as its self-test
# does with each fixture. The head defaults to HEAD. Without a base, the merge base of head with
# origin/main (else main) is used: the commits of the branch not yet on main. CI passes the pull
# request's base, or the previous tip on a push to main, none after a force push; a base of zeros,
# which a push reports where there was no previous tip, counts as none.
# Exit code 0 when no commit changes an accepted ADR's text, 1 when one does, 2 on a base or head
# that does not resolve.

set -uo pipefail

# shellcheck source-path=SCRIPTDIR source=lib/git.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/git.sh"
# shellcheck source-path=SCRIPTDIR source=lib/adr.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/adr.sh"

enter_root "Accepted ADR text" "${BASH_SOURCE[0]}" "$@"
shift "$ROOT_TAKEN"

base="${1:-}"
head="${2:-HEAD}"

if [[ -f TEMPLATE-SETUP.md ]]; then
  echo "Accepted ADR text: the template repository corrects its own ADRs in place (ADR-0005)."
  exit 0
fi

[[ "$base" =~ ^0+$ ]] && base=""
[[ -n "$base" ]] || base="$(main_merge_base "$head")"
if [[ -z "$base" ]]; then
  echo "Accepted ADR text: nothing to compare — no base given and no main to compare with."
  exit 0
fi
for ref in "$base" "$head"; do
  git rev-parse --verify -q "$ref^{commit}" >/dev/null \
    || { echo "Accepted ADR text: '$ref' does not resolve to a commit." >&2; exit 2; }
done

# bound <commit> <path> — whether the ADR at the path is accepted or superseded at the commit.
bound() {
  git cat-file -e "$1:$2" 2>/dev/null || return 1
  [[ "$(git show "$1:$2" | adr_field_line - Status)" =~ (accepted|superseded) ]]
}

# header_end <commit> <path> — the number of the ADR's first '## ' line at the commit; the lines
# above it are its header.
header_end() {
  git show "$1:$2" | awk '/^## / { print NR; found = 1; exit } END { if (!found) print NR + 1 }'
}

# changed <before> <commit> <path> — the lines of the ADR the commit changes beyond those it may,
# as '<number of lines>\t<line number>: <the first of them>', or nothing. A line is the old
# version's where removed and the new one's where added; a line of the header may be its Date
# line, its Status line flipped to superseded, or its Deciders line with the old one's names kept
# at its start.
changed() {
  local before="$1" c="$2" path="$3" diff
  if git rev-parse -q --verify "$c^2" >/dev/null; then
    diff="$(git show --remerge-diff --format= -p -U0 --no-color --no-renames "$c" -- "$path")"
  else
    diff="$(git diff -p -U0 --no-color --no-renames "$before" "$c" -- "$path")"
  fi
  awk -v old_end="$(header_end "$before" "$path")" -v new_end="$(header_end "$c" "$path")" '
    /^diff --git / { hunk = 0; next }
    /^@@ / {
      hunk = 1
      split($2, o, ","); split($3, n, ",")
      old = substr(o[1], 2) + 0; new = substr(n[1], 2) + 0
      # A hunk that removes no line, or adds none, names the line before it.
      if (o[2] == "0") old++
      if (n[2] == "0") new++
      next
    }
    !hunk { next }
    /^-/ {
      line = substr($0, 2)
      if (old < old_end && line ~ /^- \*\*(Date|Status):\*\*/) { old++; next }
      if (old < old_end && line ~ /^- \*\*Deciders:\*\*/) { deciders = line; old++; next }
      report(old, $0); old++; next
    }
    /^\+/ {
      line = substr($0, 2)
      if (new < new_end && line ~ /^- \*\*Date:\*\*/) { new++; next }
      if (new < new_end && line ~ /^- \*\*Status:\*\* .*superseded/) { new++; next }
      if (new < new_end && line ~ /^- \*\*Deciders:\*\*/ && deciders != "" &&
          substr(line, 1, length(deciders)) == deciders) { new++; next }
      report(new, $0); new++; next
    }
    function report(at, text) { if (!count++) first = at ": " text }
    END { if (count) printf "%d\t%s\n", count, first }
  ' <<< "$diff"
}

findings=()
trailed=()
while IFS= read -r c; do
  short="$(git rev-parse --short "$c")"
  before="$(git rev-parse -q --verify "$c^1" 2>/dev/null)" || before=""
  [[ -n "$before" ]] || continue
  hits=()
  while IFS= read -r -d '' status && IFS= read -r -d '' path; do
    [[ "$path" =~ ^docs/adr/[0-9]{4}-[^/]*\.md$ ]] || continue
    bound "$before" "$path" || continue
    adr="ADR-$(adr_key "$path")"
    if [[ "$status" == D ]]; then
      hits+=("$short deletes $adr ($path), which is accepted")
      continue
    fi
    found="$(changed "$before" "$c" "$path")"
    [[ -n "$found" ]] || continue
    n="${found%%$'\t'*}"
    hits+=("$short changes $n line$( ((n == 1)) || echo s) of $adr ($path), which is accepted, the first at line ${found#*$'\t'}")
  done < <(git -c core.quotePath=false diff-tree -r -z --no-commit-id --no-renames --name-status \
    "$before" "$c" -- docs/adr/)
  ((${#hits[@]})) || continue
  if git log -1 --format='%(trailers:key=Normalization,valueonly)' "$c" | grep -q '[^[:space:]]'; then
    trailed+=("$short $(git log -1 --format=%s "$c")")
  else
    findings+=("${hits[@]}")
  fi
done < <(git rev-list --reverse "$base..$head")

if ((${#trailed[@]})); then
  echo "Accepted ADR text: these commits rewrite accepted ADRs as a normalization, by their trailer:"
  printf '  - %s\n' "${trailed[@]}"
fi
if ((${#findings[@]})); then
  echo "Accepted ADR text: an accepted ADR is changed only by superseding it, outside a" \
    "normalization (AGENTS.md, section 3, rule 5):" >&2
  printf '  - %s\n' "${findings[@]}" >&2
  exit 1
fi
echo "Accepted ADR text: no accepted ADR's text changed since $(git rev-parse --short "$base")."
exit 0
