#!/usr/bin/env bash
#
# Measures Q-3 (feature spec #11, #16): the memory the desktop application takes once it shows the
# commit graph of a repository of 100 000 commits.
#
# The repository is generated, the same each time: a main line with a feature branch of five
# commits merged back every fifty. The application starts on it under a virtual display. Once it
# reports the loaded graph on standard error, the proportional set size (PSS) of the application
# and every process it starts, the WebKit processes included, is summed once a second until the
# sum stops changing. An application that never reports a loaded graph fails the run: its memory
# would be that of an empty window. The application lays out at most the commit limit of the
# repository (ADR-0016), so most of the memory is the application itself. The run measures
# twice: when the two sums differ by 10 percent or more it reports the spread and gives no
# verdict; otherwise it says whether Q-3 holds. While Q-3 is missed the script only reports, as
# decided with the maintainer in #16: it fails when the memory cannot be measured or the runs
# spread, not when the limit is missed.
#
# Usage:
#     scripts/measure-memory.sh
#     scripts/measure-memory.sh --verdict FIRST_KB SECOND_KB   (only the verdict, for the self-test)
# Environment, all optional: OCELLI_DESKTOP, the application (built here with the Tauri CLI when
# unset); OCELLI_MEMORY_COMMITS, the size of the repository (100000); OCELLI_MEMORY_LIMIT_MB, the
# limit (200); OCELLI_MEMORY_WRAPPER, what starts the application on a display ("xvfb-run -a");
# OCELLI_MEMORY_SETTLE, the seconds of samples taken before the sum may count as settled (10);
# OCELLI_MEMORY_TIMEOUT, the seconds a run may take (180).
# Exit code 0 when the memory was measured, 1 when it could not be or the runs spread.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
commits="${OCELLI_MEMORY_COMMITS:-100000}"
limit_mb="${OCELLI_MEMORY_LIMIT_MB:-200}"
wrapper="${OCELLI_MEMORY_WRAPPER-xvfb-run -a}"
settle="${OCELLI_MEMORY_SETTLE:-10}"
timeout="${OCELLI_MEMORY_TIMEOUT:-180}"

# judge <first kB> <second kB> — set verdict and rc from the two runs and the limit: no verdict
# and a failed run when they differ by 10 percent or more of the larger, else whether the larger
# stays below the limit, which is only reported.
judge() {
  local high=$(($1 > $2 ? $1 : $2)) low=$(($1 > $2 ? $2 : $1))
  if (((high - low) * 10 >= high)); then
    verdict="no verdict: the runs differ by 10 percent or more"
    rc=1
  elif ((high >= limit_mb * 1024)); then
    verdict="missed"
    rc=0
  else
    verdict="holds"
    rc=0
  fi
}

# With --verdict, only judge two given runs, so that the self-test can try the verdict at its
# boundaries without starting an application.
if [[ "${1:-}" == --verdict ]]; then
  judge "$2" "$3"
  echo "$verdict"
  exit "$rc"
fi

app="${OCELLI_DESKTOP:-}"
if [[ -z "$app" ]]; then
  (cd "$ROOT" && bun run tauri build --no-bundle -- --locked >/dev/null) \
    || { echo "memory: cannot build the desktop application" >&2; exit 1; }
  app="$ROOT/target/release/ocelli"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# A signal stops the application and its display with the script: they share its process group.
trap 'trap - INT TERM; kill 0; exit 1' INT TERM

# generate <dir> <commits> — a repository whose history is a main line with a feature branch of
# five commits merged back every fifty, written through fast-import at fixed times.
generate() {
  git init --quiet --bare "$1"
  awk -v n="$2" 'BEGIN {
    t = 1700000000; mark = 0; main = 0
    for (i = 1; i <= n; i++) {
      if (i % 50 == 0 && main > 0 && i + 6 <= n) {
        base = main
        for (j = 1; j <= 5; j++) {
          printf "commit refs/heads/feature\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 7\nfeature\nfrom :%d\n\n", ++mark, ++t, (j == 1 ? base : mark - 1)
          i++
        }
        printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 5\nmerge\nfrom :%d\nmerge :%d\n\n", ++mark, ++t, main, mark - 1
        main = mark
        continue
      }
      printf "commit refs/heads/main\nmark :%d\ncommitter F <f@example.invalid> %d +0000\ndata 4\nmain\n", ++mark, ++t
      if (main > 0) printf "from :%d\n", main
      printf "\n"
      main = mark
    }
  }' | git -C "$1" fast-import --quiet
  git -C "$1" symbolic-ref HEAD refs/heads/main
}

# descendants <pid> — the pid and every process below it.
descendants() {
  local pid="$1" child
  echo "$pid"
  for child in $(ps -o pid= --ppid "$pid" 2>/dev/null); do
    descendants "$child"
  done
}

# kill_tree <pid> — stop the process and everything below it, the deepest first.
# shellcheck disable=SC2317  # called through the RETURN trap of measure
kill_tree() {
  local p
  for p in $(descendants "$1" | tac); do
    kill "$p" 2>/dev/null
  done
  wait "$1" 2>/dev/null
}

# pss_kb <pid> — the summed PSS of the application below the started process, the display
# server left out.
pss_kb() {
  local sum=0 p comm kb
  for p in $(descendants "$1"); do
    comm="$(cat "/proc/$p/comm" 2>/dev/null)" || continue
    [[ "$comm" == Xvfb || "$comm" == xvfb-run ]] && continue
    kb="$(awk '/^Pss:/ { print $2 }' "/proc/$p/smaps_rollup" 2>/dev/null)"
    sum=$((sum + ${kb:-0}))
  done
  echo "$sum"
}

# measure — start the application on the repository and print the commits it loaded and its
# settled PSS in kB. It runs in a subshell, so it stops the application itself on every way out.
measure() {
  local pid samples=() kb low high i loaded window=$((settle < 5 ? settle : 5))
  # shellcheck disable=SC2086  # the wrapper is a command and its arguments
  $wrapper "$app" "$work/repo.git" >"$work/app.log" 2>&1 &
  pid=$!
  trap 'kill_tree "$pid"' RETURN
  for ((i = 0; i < timeout; i++)); do
    sleep 1
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "memory: the application exited:" >&2
      sed 's/^/    /' "$work/app.log" >&2
      return 1
    fi
    # The status line the desktop host writes after a load, loaded_status in
    # src-tauri/src/commands.rs, whose tests hold this shape.
    loaded="$(grep -m 1 -oE '^[0-9]+ commits' "$work/app.log" | grep -oE '^[0-9]+')"
    [[ -z "$loaded" ]] && continue
    kb="$(pss_kb "$pid")"
    samples+=("$kb")
    ((${#samples[@]} < settle)) && continue
    low="$(printf '%s\n' "${samples[@]: -window}" | sort -n | head -n 1)"
    high="$(printf '%s\n' "${samples[@]: -window}" | sort -n | tail -n 1)"
    if ((high > 0 && (high - low) * 100 <= high * 2)); then
      echo "$loaded $high"
      return 0
    fi
  done
  if [[ -z "$loaded" ]]; then
    echo "memory: the application never reported a loaded graph within $timeout seconds:" >&2
    sed 's/^/    /' "$work/app.log" >&2
  else
    echo "memory: the memory did not settle within $timeout seconds" >&2
  fi
  return 1
}

generate "$work/repo.git" "$commits" || { echo "memory: cannot generate the repository" >&2; exit 1; }
read -r loaded first < <(measure) && [[ -n "${first:-}" ]] || exit 1
read -r _ second < <(measure) && [[ -n "${second:-}" ]] || exit 1
mb() { awk -v kb="$1" 'BEGIN { printf "%.1f", kb / 1024 }'; }
first_mb="$(mb "$first")"
second_mb="$(mb "$second")"
judge "$first" "$second"

report="| Commits | Laid out | First run | Second run | Limit | Q-3 |
|---|---|---|---|---|---|"
report+=$'\n'"| $commits | $loaded | $first_mb MB | $second_mb MB | $limit_mb MB | $verdict |"

echo "$report"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  { echo "## Q-3: memory of the desktop application"; echo; echo "$report"; } >> "$GITHUB_STEP_SUMMARY"
fi
exit "$rc"
