#!/usr/bin/env bash
#
# Measures Q-3 (feature spec #11): what showing the commit graph of a repository of 100 000 commits
# adds to the memory of the desktop application, its WebView's processes included, over the
# application with no repository open, on Linux. Verifies: Q-3
#
# The repository is generated, the same each time: a main line with a feature branch of five
# commits merged back every fifty. The application starts under a virtual display, once with no
# repository and once on the repository; the proportional set size (PSS) of the application and
# every process it starts, the WebKit processes included, is summed once a second until the sum
# stops changing. On the repository the sampling starts only once the application reports the
# loaded graph on standard error; an application that never does fails the run, since its memory
# would be that of an empty window. Each is measured twice. When the two runs of either differ by
# 10 percent or more the run reports the spread and fails without a verdict. Otherwise the memory
# added is the larger loaded sum less the smaller empty one, and Q-3 holds below the limit; at or
# above it the run fails. The absolute memory is reported beside it and decides nothing: whether
# the desktop host keeps its own footprint is ADR-0018's, which no test decides.
#
# Usage:
#     scripts/measure-memory.sh
#     scripts/measure-memory.sh --verdict EMPTY_KB EMPTY_KB LOADED_KB LOADED_KB   (the verdict only)
# Environment, all optional: OCELLI_DESKTOP, the application (built here with the Tauri CLI when
# unset); OCELLI_MEMORY_COMMITS, the size of the repository (100000); OCELLI_MEMORY_LIMIT_MB, the
# memory the graph may add (150); OCELLI_MEMORY_WRAPPER, what starts the application on a display
# ("xvfb-run -a"); OCELLI_MEMORY_SETTLE, the seconds of samples taken before a sum may count as
# settled (10); OCELLI_MEMORY_TIMEOUT, the seconds a run may take (180).
# Exit code 0 when Q-3 holds, 1 when it is missed or the memory cannot be measured.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
commits="${OCELLI_MEMORY_COMMITS:-100000}"
limit_mb="${OCELLI_MEMORY_LIMIT_MB:-150}"
wrapper="${OCELLI_MEMORY_WRAPPER-xvfb-run -a}"
settle="${OCELLI_MEMORY_SETTLE:-10}"
timeout="${OCELLI_MEMORY_TIMEOUT:-180}"

# spread <a kB> <b kB> — whether two runs differ by 10 percent or more of the larger.
spread() {
  local high=$(($1 > $2 ? $1 : $2)) low=$(($1 > $2 ? $2 : $1))
  (((high - low) * 10 >= high))
}

# judge <empty kB> <empty kB> <loaded kB> <loaded kB> — set added (kB), verdict, and rc: no verdict
# and a failed run when either pair spreads, else whether the added memory stays below the limit.
judge() {
  local empty=$(($1 < $2 ? $1 : $2)) loaded=$(($3 > $4 ? $3 : $4))
  added=$((loaded > empty ? loaded - empty : 0))
  if spread "$1" "$2" || spread "$3" "$4"; then
    verdict="no verdict: the runs differ by 10 percent or more"
    rc=1
  elif ((added >= limit_mb * 1024)); then
    verdict="missed"
    rc=1
  else
    verdict="holds"
    rc=0
  fi
}

# With --verdict, only judge four given runs, so that the self-test can try the verdict at its
# boundaries without starting an application.
if [[ "${1:-}" == --verdict ]]; then
  judge "$2" "$3" "$4" "$5"
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

# measure [repository] — start the application, on the repository when one is given, and print
# the commits it loaded (0 without one) and its settled PSS in kB. It runs in a subshell, so it
# stops the application itself on every way out.
measure() {
  local pid samples=() kb low high i loaded=0 window=$((settle < 5 ? settle : 5))
  # shellcheck disable=SC2086  # the wrapper is a command and its arguments
  $wrapper "$app" "$@" >"$work/app.log" 2>&1 &
  pid=$!
  trap 'kill_tree "$pid"' RETURN
  for ((i = 0; i < timeout; i++)); do
    sleep 1
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "memory: the application exited:" >&2
      sed 's/^/    /' "$work/app.log" >&2
      return 1
    fi
    if (($#)); then
      # The status line the desktop host writes after a load, loaded_status in
      # src-tauri/src/commands.rs, whose tests hold this shape.
      loaded="$(grep -m 1 -oE '^[0-9]+ commits' "$work/app.log" | grep -oE '^[0-9]+')"
      [[ -z "$loaded" ]] && continue
    fi
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

bash "$ROOT/scripts/generate-history.sh" "$work/repo.git" "$commits" || { echo "memory: cannot generate the repository" >&2; exit 1; }
read -r _ empty1 < <(measure) && [[ -n "${empty1:-}" ]] || exit 1
read -r _ empty2 < <(measure) && [[ -n "${empty2:-}" ]] || exit 1
read -r loaded loaded1 < <(measure "$work/repo.git") && [[ -n "${loaded1:-}" ]] || exit 1
read -r _ loaded2 < <(measure "$work/repo.git") && [[ -n "${loaded2:-}" ]] || exit 1
judge "$empty1" "$empty2" "$loaded1" "$loaded2"

mb() { awk -v kb="$1" 'BEGIN { printf "%.1f", kb / 1024 }'; }
report="| Commits | Laid out | Empty | Loaded | Added | Limit | Q-3 |
|---|---|---|---|---|---|---|"
report+=$'\n'"| $commits | $loaded | $(mb "$empty1") / $(mb "$empty2") MB | $(mb "$loaded1") / $(mb "$loaded2") MB | $(mb "$added") MB | $limit_mb MB | $verdict |"

echo "$report"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  { echo "## Q-3: memory the commit graph adds to the desktop application"; echo; echo "$report"; } \
    >> "$GITHUB_STEP_SUMMARY"
fi
exit "$rc"
