#!/usr/bin/env bash
#
# Self-test of the git hooks under .githooks/: pre-commit refuses a commit on main, pre-push
# refuses a push while the checks are red, while the working tree differs from HEAD, of a ref
# that is not the checked-out commit, and while the last completed CI run of the branch pushed to
# is red unless OCELLI_PUSH_OVER_RED=1 is set; where gh cannot tell, the push goes on (AGENTS.md,
# sections 5 and 6).
#
# Each case builds a throwaway repository under a temporary directory with core.hooksPath pointing
# at this repository's .githooks/, runs a real 'git commit' or 'git push' in it — the push goes to
# a bare repository next to it — and asserts the exit code and, for a refused command, the message.
# A push case ships its own scripts/check-all.sh, green or red, so the test decides what the hook
# finds. No case touches the network: a stub gh first on PATH answers as GH_STUB tells it (red,
# green, none, fail, hang), and a PATH without gh stands in for a machine that lacks it.
#
# Usage:
#     scripts/test-git-hooks.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS="$ROOT/.githooks"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failed=0
passed=0

# The stub gh. 'run list' answers only for a branch named topic*, so a case shows the hook asks
# about the branch pushed to, not the local one; 'run view' lists the jobs of the red run.
mkdir -p "$work/stub"
cat > "$work/stub/gh" <<'STUB'
#!/usr/bin/env bash
case "${GH_STUB:-none}:$1 $2" in
  fail:*) echo "gh: not logged in" >&2; exit 4 ;;
  hang:*) exec sleep 30 ;;
  red:"run list")
    [[ " $* " == *" --branch topic"* ]] && printf 'failure\t4711\thttps://example.invalid/runs/4711\n' ;;
  green:"run list")
    [[ " $* " == *" --branch topic"* ]] && printf 'success\t4712\thttps://example.invalid/runs/4712\n' ;;
  red:"run view") printf 'Build, test, lint (windows-latest)\nShell lint\n' ;;
esac
exit 0
STUB
chmod +x "$work/stub/gh"
export PATH="$work/stub:$PATH"

# A PATH without gh: every other program on PATH, linked into one directory.
mkdir -p "$work/nogh"
IFS=: read -ra path_dirs <<< "$PATH"
for pd in "${path_dirs[@]}"; do
  for f in "$pd"/*; do
    name="${f##*/}"
    [[ "$name" == gh || ! -x "$f" || -e "$work/nogh/$name" ]] && continue
    ln -s "$f" "$work/nogh/$name"
  done
done

# g <dir> <git args...> — git in the fixture with an identity, so that commits need no config.
g() {
  local dir="$1"; shift
  git -C "${dir:?}" -c user.name=t -c user.email=t@example.invalid "$@"
}

# repo <name> — a fixture repository with one commit on main, the hooks enabled after that
# commit, and a bare remote 'origin' to push to. Prints its path.
repo() {
  local dir="$work/$1"
  mkdir -p "$dir"
  git -C "${dir:?}" init -q -b main
  g "$dir" commit -q --allow-empty -m init
  git -C "${dir:?}" config core.hooksPath "$HOOKS"
  git init -q --bare "$dir.git"
  git -C "${dir:?}" remote add origin "$dir.git"
  echo "$dir"
}

# checks <dir> <exit code> — give the fixture a committed scripts/check-all.sh that exits as told.
checks() {
  mkdir -p "$1/scripts"
  printf '#!/usr/bin/env bash\necho "check output for the test"\nexit %s\n' "$2" > "$1/scripts/check-all.sh"
  g "$1" add scripts/check-all.sh
  g "$1" commit -q -m checks
}

# workflow <dir> — give the fixture a committed .github/workflows/checks.yml, so that pre-push
# asks gh about CI.
workflow() {
  mkdir -p "$1/.github/workflows"
  printf 'name: Checks\n' > "$1/.github/workflows/checks.yml"
  g "$1" add .github/workflows/checks.yml
  g "$1" commit -q -m workflow
}

# expect <pass|refuse> <name> <dir> <needle> <git args...> — run the git command; a refused one
# must exit non-zero with the needle in its output, a passing one exit zero, with the needle in
# its output when one is given.
expect() {
  local want="$1" name="$2" dir="$3" needle="$4"
  shift 4
  local out rc
  out="$(g "$dir" "$@" 2>&1)"; rc=$?
  if [[ "$want" == "pass" && $rc -eq 0 && "$out" == *"$needle"* ]] \
     || [[ "$want" == "refuse" && $rc -ne 0 && "$out" == *"$needle"* ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected $want${needle:+ with \"$needle\"}, exit $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

# --- cases ---------------------------------------------------------------------------------

d="$(repo on-main)"
expect refuse "a commit on main" "$d" "on 'main'" commit -q --allow-empty -m x
expect pass "a push from main with no check script to run" "$d" "" push -q origin main

d="$(repo on-branch)"
git -C "${d:?}" switch -q -c feature
expect pass "a commit on a branch" "$d" "" commit -q --allow-empty -m x

d="$(repo red)"
git -C "${d:?}" switch -q -c feature
checks "$d" 1
expect refuse "a push while the checks are red" "$d" "the checks are red" push -q origin feature
expect refuse "the refusal shows the check output" "$d" "check output for the test" push -q origin feature
expect pass "a commit is not gated by the checks" "$d" "" commit -q --allow-empty -m x

d="$(repo green)"
git -C "${d:?}" switch -q -c feature
checks "$d" 0
expect pass "a push while the checks are green" "$d" "" push -q origin feature

d="$(repo dirty)"
git -C "${d:?}" switch -q -c feature
checks "$d" 0
echo changed >> "$d/scripts/check-all.sh"
expect refuse "a push while a tracked file differs from HEAD" "$d" "differs from HEAD" push -q origin feature
expect refuse "the refusal names the file" "$d" "scripts/check-all.sh" push -q origin feature

d="$(repo untracked)"
git -C "${d:?}" switch -q -c feature
checks "$d" 0
echo new > "$d/new.txt"
expect refuse "a push while an untracked file lies about" "$d" "new.txt" push -q origin feature
printf 'new.txt\n' > "$d/.git/info/exclude"
expect pass "an ignored file does not count" "$d" "" push -q origin feature

d="$(repo other)"
git -C "${d:?}" switch -q -c feature
checks "$d" 0
git -C "${d:?}" branch -q other
g "$d" commit -q --allow-empty -m ahead
expect refuse "a push of a ref that is not the checked-out commit" "$d" "not the checked-out commit" push -q origin other

d="$(repo detached)"
git -C "${d:?}" checkout -q --detach
expect pass "a commit on a detached HEAD is not on main" "$d" "" commit -q --allow-empty -m x

d="$(repo ci)"
git -C "${d:?}" switch -q -c feature
checks "$d" 0
workflow "$d"
GH_STUB=red expect refuse "a push while the branch's CI is red" "$d" "is red (failure): run 4711" push -q origin feature:topic
GH_STUB=red expect refuse "the refusal names the run" "$d" "https://example.invalid/runs/4711" push -q origin feature:topic
GH_STUB=red expect refuse "the refusal names the failed jobs" "$d" "Build, test, lint (windows-latest)" push -q origin feature:topic
GH_STUB=red expect refuse "the refusal names the override" "$d" "OCELLI_PUSH_OVER_RED=1 git push" push -q origin feature:topic
GH_STUB=red OCELLI_PUSH_OVER_RED=1 expect pass "the override pushes over red CI" "$d" "pushing anyway" push -q origin feature:topic-over
GH_STUB=red expect pass "red CI of another branch does not count" "$d" "" push -q origin feature
GH_STUB=green expect pass "a push while the branch's CI is green" "$d" "" push -q origin feature:topic-green
GH_STUB=none expect pass "no completed run yet: the hook could not tell" "$d" "no completed checks.yml run yet" push -q origin feature:topic-none
GH_STUB=fail expect pass "gh failing, as when not authenticated: the hook could not tell" "$d" "gh failed" push -q origin feature:topic-fail
GH_STUB=hang OCELLI_PUSH_CI_TIMEOUT=1 expect pass "gh hanging, as when offline: the hook gives up" "$d" "took longer than 1s" push -q origin feature:topic-hang
PATH="$work/nogh" expect pass "gh missing: the hook could not tell" "$d" "gh is not installed" push -q origin feature:topic-nogh

# --- summary -------------------------------------------------------------------------------

if ((failed)); then
  echo "git hooks self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "git hooks self-test passed ($passed cases)."
exit 0
