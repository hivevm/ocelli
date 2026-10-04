#!/usr/bin/env bash
#
# Measures Q-1 on the corpus of feature spec #11: for each repository, the edge crossings in
# Ocelli's layout and in `git log --graph` over the same newest commits, by ocelli-measure.
#
# Each repository is fetched at a fixed commit as a single ref, history only and no file contents,
# so the measurement sees the same commits every week. The script reports a table, and appends it
# to the job summary when GITHUB_STEP_SUMMARY is set. While Q-1 is missed it only reports, as
# decided with the maintainer in #14: it fails only when a repository cannot be fetched or
# measured.
#
# Usage:
#     scripts/measure-crossings.sh [CORPUS]
# CORPUS is a file of lines '<name> <url> <commit>' and defaults to the corpus below. The binary
# is OCELLI_MEASURE, or built here with cargo when that is unset.
# Exit code 0 when every repository was measured, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DEFAULT_CORPUS="git https://github.com/git/git.git 8103b446517e0c44e67561b9d0ccce56efa60a71
cargo https://github.com/rust-lang/cargo.git bb2126cffae48394a37db8728dc50a17bbfe54d4
svelte https://github.com/sveltejs/svelte.git 7c2fcdae6a29fb3b36855dc8e86eb7ce4c56b93a
nuc https://github.com/hivevm/nuc.git 8322f679e7b9b61579aaf1a8d99774b6d8a79e51"

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

table="| Repository | Commit | Commits | Ocelli crossings | git crossings | Q-1 |
|---|---|---|---|---|---|"
failed=()

while read -r name url commit; do
  [[ -z "${name:-}" || "$name" == \#* ]] && continue
  repo="$work/$name.git"
  if ! git init --quiet --bare "$repo" \
    || ! git -C "$repo" fetch --quiet --no-tags --filter=tree:0 "$url" "$commit:refs/heads/measured" 2>"$work/$name.err"; then
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
  if ! [[ "$commits" =~ ^[0-9]+$ && "$ocelli" =~ ^[0-9]+$ && "$git_count" =~ ^[0-9]+$ ]]; then
    echo "measure: cannot read the counts of $name from:" >&2
    printf '    %s\n' "$out" >&2
    failed+=("$name")
    continue
  fi
  if ((ocelli < git_count)); then verdict="holds"; else verdict="missed"; fi
  table+=$'\n'"| $name | ${commit:0:7} | $commits | $ocelli | $git_count | $verdict |"
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
echo "Measured every repository of the corpus."
