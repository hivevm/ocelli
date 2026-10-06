#!/usr/bin/env bash
#
# Measures Q-1 on the corpus of feature spec #11: for each repository, the edge crossings in
# Ocelli's layout, by ocelli-measure, against the count recorded for its fixed commit.
# Verifies: Q-1, ADR-0032
#
# Each repository is fetched at a fixed commit as a single ref, history only and no file contents,
# so the measurement sees the same commits every week; the ref is checked out, as in a clone, so
# the layout measured carries its main line in the leftmost lane, as the front ends draw it. Q-1 holds when Ocelli's layout draws no more
# edge crossings than recorded; git's count over the same commits, which the straight-branch layout
# trades crossings against, is reported beside it and decides nothing. The script reports a table,
# and appends it to the job summary when GITHUB_STEP_SUMMARY is set. It fails when Q-1 is missed on
# any repository, or a repository cannot be fetched or measured.
#
# Usage:
#     scripts/measure-crossings.sh [CORPUS]
# CORPUS is a file of lines '<name> <url> <commit> <recorded crossings>' and defaults to the corpus
# below; a recorded count changes only with a decision about the layout. The binary
# is OCELLI_MEASURE, or built here with cargo when that is unset.
# Exit code 0 when every repository was measured and Q-1 holds on each, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DEFAULT_CORPUS="git https://github.com/git/git.git 8103b446517e0c44e67561b9d0ccce56efa60a71 45086
cargo https://github.com/rust-lang/cargo.git bb2126cffae48394a37db8728dc50a17bbfe54d4 1627
svelte https://github.com/sveltejs/svelte.git 7c2fcdae6a29fb3b36855dc8e86eb7ce4c56b93a 0
nuc https://github.com/hivevm/nuc.git 8322f679e7b9b61579aaf1a8d99774b6d8a79e51 0"

if [[ $# -gt 0 ]]; then
  corpus="$(cat "$1")" || exit 1
else
  corpus="$DEFAULT_CORPUS"
fi

measure="${OCELLI_MEASURE:-}"
if [[ -z "$measure" ]]; then
  cargo build --release --locked --quiet --manifest-path "$ROOT/Cargo.toml" -p ocelli-measure \
    || { echo "measure: cannot build ocelli-measure" >&2; exit 1; }
  measure="$ROOT/target/release/ocelli-measure"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

table="| Repository | Commit | Commits | Ocelli crossings | Recorded | git crossings | Q-1 |
|---|---|---|---|---|---|---|"
failed=()
missed=()

while read -r name url commit recorded; do
  [[ -z "${name:-}" || "$name" == \#* ]] && continue
  repo="$work/$name.git"
  if ! git init --quiet --bare "$repo" \
    || ! git -C "$repo" fetch --quiet --no-tags --filter=tree:0 "$url" "$commit:refs/heads/measured" 2>"$work/$name.err" \
    || ! git -C "$repo" symbolic-ref HEAD refs/heads/measured 2>>"$work/$name.err"; then
    echo "measure: cannot fetch $name at $commit:" >&2
    sed 's/^/    /' "$work/$name.err" >&2
    failed+=("$name")
    continue
  fi
  if ! out="$("$measure" crossings "$repo" 2>&1)"; then
    echo "measure: cannot measure $name:" >&2
    printf '    %s\n' "$out" >&2
    failed+=("$name")
    continue
  fi
  commits="$(sed -n 's/^commits: //p' <<< "$out")"
  ocelli="$(sed -n 's/^ocelli crossings: //p' <<< "$out")"
  git_count="$(sed -n 's/^git crossings: //p' <<< "$out")"
  if ! [[ "${recorded:-}" =~ ^[0-9]+$ ]]; then
    echo "measure: the corpus records no crossing count for $name" >&2
    failed+=("$name")
    continue
  fi
  if ! [[ "$commits" =~ ^[0-9]+$ && "$ocelli" =~ ^[0-9]+$ && "$git_count" =~ ^[0-9]+$ ]]; then
    echo "measure: cannot read the counts of $name from:" >&2
    printf '    %s\n' "$out" >&2
    failed+=("$name")
    continue
  fi
  if ((ocelli <= recorded)); then
    verdict="holds"
  else
    verdict="missed"
    missed+=("$name")
  fi
  table+=$'\n'"| $name | ${commit:0:7} | $commits | $ocelli | $recorded | $git_count | $verdict |"
done <<< "$corpus"

echo "$table"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  { echo "## Q-1: edge crossings, Ocelli against git log --graph"; echo; echo "$table"; } \
    >> "$GITHUB_STEP_SUMMARY"
fi

if ((${#failed[@]})); then
  echo "Measurement FAILED for: ${failed[*]}" >&2
  exit 1
fi
if ((${#missed[@]})); then
  echo "Q-1 MISSED on: ${missed[*]} (more edge crossings than recorded)" >&2
  exit 1
fi
echo "Measured every repository of the corpus; Q-1 holds on each."
