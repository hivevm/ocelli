#!/usr/bin/env bash
#
# Self-test of the template setup (TEMPLATE-SETUP.md) against this repository's own files.
#
# The setup promises two things the fixture tests of the single checks cannot see, because they
# hold only on the real scaffold: that check 12 and check 13 of scripts/check-docs.sh name every
# placeholder the setup leaves behind once its file is gone, and that a project which has done
# every step is green. Each case copies the repository's files into a throwaway git repository
# and runs the checks against the copy:
#
#   1. Setup file deleted, nothing else done: check-docs.sh names each placeholder of steps 1,
#      2, 3, and 5 that is still in the tree, and the README's notes that point at the setup
#      file while they are there. Every file that carries a token of a placeholder (TOKENS) is
#      named in its output, so a placeholder added to a file no row knows of fails here.
#   2. The remaining steps done, as an agent would do them from the steps' wording: no token is
#      left, and check-docs.sh, check-traceability.sh, and check-devcontainer.sh pass.
#   3. The same, on the path 'No Dev Container?' of 'A small project': ADR-0004 superseded and
#      what the container brings removed as the path lists it. check-docs.sh and
#      check-traceability.sh pass. It runs while the path is open, before a project has begun it.
#
# A project partway through the setup has replaced some placeholders already. Case 1 asks only
# for those still present, and an edit of case 2 that finds nothing to change does nothing, so
# the test holds at every point of the setup. A placeholder the scaffold rewords so that an edit
# no longer applies stays in the copy, and case 2 fails on it.
#
# The setup file goes together with this script, its 'run' line in scripts/check-all.sh, and its
# step in the docs job. Should the script outlive the file, it says there is nothing left to
# test and passes.
#
# Pure bash + coreutils/sed/tar, plus git to enumerate the files and build the copy. Usage:
#     scripts/test-template-setup.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -f "$ROOT/TEMPLATE-SETUP.md" ]]; then
  echo "Template setup self-test: TEMPLATE-SETUP.md is gone, the setup is done — nothing to test."
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failed=0
passed=0

# fixture <dir> — refuse an empty fixture path: 'git -C ""' would act on this repository.
fixture() { [[ -n "${1:-}" && -d "$1/.git" ]] || { echo "fixture: '$1' is no fixture repository" >&2; exit 1; }; }

# copy <name> — the repository's files (tracked, and new ones git does not ignore) in a fresh git
# repository, printed as its path. Symlinks stay symlinks.
copy() {
  local dir="$work/$1"
  mkdir -p "$dir"
  git -C "$ROOT" ls-files --cached --others --exclude-standard -z \
    | (cd "$ROOT" && tar --null -T - -cf - 2>/dev/null) | tar -xf - -C "$dir"
  git -C "${dir:?}" init -q -b setup
  echo "$dir"
}

# stage <dir> — let the checks see the copy's files the way they see a working tree.
stage() { fixture "$1"; git -C "${1:?}" add -A; }

# edit <dir> <file> <sed script> — apply one edit of the setup to the copy, where the file exists.
edit() { [[ ! -f "$1/$2" ]] || sed -i "$3" "$1/$2"; }

# run_check <dir> <script> — run one check against the copy; prints its output and exit code.
run_check() { fixture "$1"; bash "$ROOT/scripts/$2" "$1" 2>&1; }

# expect_pass <name> <dir> <script> — the check must pass on the copy.
expect_pass() {
  local name="$1" dir="$2" script="$3" out rc
  out="$(run_check "$dir" "$script")"; rc=$?
  if ((rc == 0)); then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name ($script exited $rc)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

# --- case 1: the setup file deleted, nothing else done ----------------------------------------

# TOKENS — what marks a placeholder of the scaffold; SCAFFOLD — the files that may carry one.
# Both are the template's own: a project's files, and a TODO or an owner of its own, are not
# read, so the scan holds while a project is partway through the setup.
TOKENS='<Project Name>|TODO: add a |^- \*\*<[A-Z][a-z]+>\*\*|@owner$|\*\*NUC\*\*|NUC DevContainer|^Describe what this project|^TODO — show'
SCAFFOLD='^(README|CONTRIBUTING|SECURITY|CODE_OF_CONDUCT|AGENTS)\.md$|^LICENSE$|^docs/[A-Z]+\.md$|^\.github/CODEOWNERS$|^\.devcontainer/devcontainer\.json$'

# scaffold_tokens <dir> — every file of the tree that still carries a token, one per line.
scaffold_tokens() {
  local f
  while IFS= read -r f; do
    [[ -f "$1/$f" ]] && grep -qE "$TOKENS" "$1/$f" && echo "$f"
  done < <(git -C "${1:?}" ls-files --cached --others --exclude-standard | grep -E "$SCAFFOLD")
}

# Each row: the file, a pattern that finds the placeholder in it, and the message check-docs.sh
# reports while the placeholder is there. The pattern may hold a '|'; the message holds none.
PLACEHOLDERS=(
  "README.md|^# NUC — |README.md: the title is still the template's"
  "README.md|^\*\*NUC\*\* is named after|README.md: the intro still explains the template's name"
  "README.md|^Describe what this project does|README.md: the Overview is still the scaffold's sentence"
  "README.md|^TODO — show a minimal example|README.md: the Usage section is still TODO"
  "README.md|github\.com/hivevm/nuc/actions/workflows/|README.md: the badge still points at the template's repository"
  ".devcontainer/devcontainer.json|\"name\": \"NUC DevContainer\"|devcontainer.json: the Dev Container still carries the template's name"
  "LICENSE|^Copyright .* Maintainer$|LICENSE: the copyright holder is still the placeholder"
  "SECURITY.md|TODO: add a security contact|SECURITY.md: the security contact is still the placeholder"
  "CODE_OF_CONDUCT.md|TODO: add a contact address|CODE_OF_CONDUCT.md: the reporting contact is still the placeholder"
  ".github/CODEOWNERS|^# /docs/|.github/CODEOWNERS: no active rule"
  ".github/CODEOWNERS|^# TODO: add a code owner|.github/CODEOWNERS: the instruction to add a code owner is still there"
  "docs/SPECIFICATION.md|^# .*<Project Name>|docs/SPECIFICATION.md: the title still carries '<Project Name>'"
  "docs/ARCHITECTURE.md|^# .*<Project Name>|docs/ARCHITECTURE.md: the title still carries '<Project Name>'"
  "docs/GLOSSARY.md|^# .*<Project Name>|docs/GLOSSARY.md: the title still carries '<Project Name>'"
  "docs/CONVENTIONS.md|^# .*<Project Name>|docs/CONVENTIONS.md: the title still carries '<Project Name>'"
  "docs/SPECIFICATION.md|\*\*[GQ]-[0-9]+\*\* — (…|<Quality)|docs/SPECIFICATION.md: a criterion is still the scaffold's placeholder"
  "docs/ARCHITECTURE.md|^- \*\*<Part>\*\*|docs/ARCHITECTURE.md: the scaffold's building block is still listed"
  "docs/GLOSSARY.md|^- \*\*<Term>\*\*|docs/GLOSSARY.md: the scaffold's entry is still listed"
  "docs/CONVENTIONS.md|^- \*\*<Convention>\*\*|docs/CONVENTIONS.md: the scaffold's entry is still listed"
  "README.md|^- \*\*(Build|Test|Lint|Run):\*\* TODO|README.md: a Build, Test, Lint, or Run command is still TODO"
  "docs/adr/0001-agent-governance-model.md|^- \*\*Deciders:\*\* NUC maintainer$|inherited from the template and not decided by this project"
  "README.md|\(TEMPLATE-SETUP\.md\)|README.md: broken relative link -> 'TEMPLATE-SETUP.md'"
  "README.md|^TEMPLATE-SETUP\.md |README.md: Project Layout lists 'TEMPLATE-SETUP.md'"
)

d="$(copy pending)"
rm "$d/TEMPLATE-SETUP.md"
stage "$d"
out="$(run_check "$d" check-docs.sh)"
for row in "${PLACEHOLDERS[@]}"; do
  file="${row%%|*}"
  rest="${row#*|}"
  message="${rest##*|}"
  pattern="${rest%|*}"
  if [[ ! -f "$ROOT/$file" ]] || ! grep -qE "$pattern" "$ROOT/$file"; then continue; fi
  if [[ "$out" == *"$message"* ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: once the setup file is gone, check-docs.sh does not report: $message" >&2
  fi
done
# Every file that carries a token must be named by check-docs.sh: a placeholder in a file no row
# above knows of is one check 13 does not hold either.
while IFS= read -r file; do
  if [[ "$out" == *"$file: "* ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $file carries a placeholder of the scaffold, and check-docs.sh names no placeholder in it" >&2
  fi
done < <(scaffold_tokens "$ROOT")

# --- case 2: every step done ------------------------------------------------------------------

# setup_steps <dir> — the steps of the setup, done in the copy as an agent would do them.
setup_steps() {
  local d="$1" f doc
  rm "$d/TEMPLATE-SETUP.md"
  # The README's notes that point at the setup file, and its row in the Project Layout block.
  edit "$d" README.md '/^> \[!NOTE\]$/,/^$/d'
  edit "$d" README.md '/^> Setting the project up/,/^$/d'
  edit "$d" README.md '/^TEMPLATE-SETUP\.md /d'
  # Step 1: the project's identity.
  edit "$d" README.md 's/^# NUC — .*/# Fixture/'
  edit "$d" README.md 's/^\*\*NUC\*\* is named after.*/**Fixture** answers questions./'
  edit "$d" .devcontainer/devcontainer.json 's/"name": "NUC DevContainer"/"name": "Fixture"/'
  edit "$d" README.md '/github\.com\/hivevm\/nuc\/actions\/workflows\//d'
  edit "$d" LICENSE 's/^\(Copyright .*\) Maintainer$/\1 Fixture Owner/'
  edit "$d" .github/CODEOWNERS 's|^# \(/docs/[^ ]*\) @owner$|\1 @fixture|'
  edit "$d" .github/CODEOWNERS '/^# TODO: add a code owner/,/^# would enforce nothing/d'
  edit "$d" SECURITY.md 's/<!-- TODO: add a security contact address -->/security@example.invalid/'
  edit "$d" CODE_OF_CONDUCT.md 's/<!-- TODO: add a contact address[^>]*-->/conduct@example.invalid/'
  # Step 2: every inherited ADR accepted by a decider of this project.
  for f in "$d"/docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
    edit "$d" "docs/adr/$(basename "$f")" \
      's/^- \*\*Deciders:\*\* NUC maintainer$/- **Deciders:** NUC maintainer, Fixture Owner/'
  done
  # Step 3: the project idea, and the documents of docs/ named and without scaffold entries.
  for doc in SPECIFICATION ARCHITECTURE GLOSSARY CONVENTIONS; do
    edit "$d" "docs/$doc.md" '1s/<Project Name>/Fixture/'
  done
  edit "$d" docs/SPECIFICATION.md \
    's/\*\*\(G-[0-9]*\)\*\* — …$/**\1** — When asked, the system shall answer./'
  edit "$d" docs/SPECIFICATION.md 's/<Quality, e\.g\. [^>]*>/Simplicity/'
  edit "$d" docs/ARCHITECTURE.md '/^- \*\*<Part>\*\*/,/^$/d'
  # shellcheck disable=SC2016  # "$" is sed's last line, not a shell expansion
  edit "$d" docs/GLOSSARY.md '/^- \*\*<Term>\*\*/,$d'
  edit "$d" docs/CONVENTIONS.md '/^- \*\*<Convention>\*\*/d'
  # Step 5: the README's Overview and Usage, and its commands; "none" where there is nothing to
  # run.
  edit "$d" README.md 's/^Describe what this project does.*/Fixture answers questions./'
  edit "$d" README.md 's/^TODO — show a minimal example.*/Run fixture ask./'
  edit "$d" README.md 's/^- \*\*\(Build\|Test\|Lint\|Run\):\*\* TODO.*/- **\1:** none/'
}

d="$(copy complete)"
setup_steps "$d"
stage "$d"
# A token left after every step is a placeholder no step names.
left="$(scaffold_tokens "$d")"
if [[ -z "$left" ]]; then
  passed=$((passed + 1))
else
  failed=$((failed + 1))
  echo "FAIL: placeholders no step of the setup replaces, in: $(tr '\n' ' ' <<< "$left")" >&2
fi
expect_pass "every step done passes the documentation checks" "$d" check-docs.sh
expect_pass "every step done passes the traceability check" "$d" check-traceability.sh
if [[ -f "$ROOT/scripts/check-devcontainer.sh" ]]; then
  expect_pass "every step done passes the Dev Container check" "$d" check-devcontainer.sh
fi

# --- case 3: every step done, on the small-project path without a Dev Container ----------------

# Only while the path is still open: a project that has walked it, or begun to, has no container
# left to remove, and case 2 covers what it has.
if [[ -d "$ROOT/.devcontainer" && -f "$ROOT/scripts/check-devcontainer.sh" ]] \
  && grep -q '^- \*\*Status:\*\* 🟢 accepted$' "$ROOT/docs/adr/0004-dev-container-runtime.md" 2>/dev/null; then
  d="$(copy no-container)"
  setup_steps "$d"
  # The superseding ADR, with the next free number; assembled, so this file cites no ADR that
  # does not exist here.
  n="$(find "$d/docs/adr" -maxdepth 1 -name '[0-9][0-9][0-9][0-9]-*.md' | wc -l)"
  id="$(printf 'ADR-%04d' "$((n + 1))")"
  file="$(printf '%04d-work-on-the-host.md' "$((n + 1))")"
  cat > "$d/docs/adr/$file" <<ADR
# $id: The work runs on the host

- **Status:** 🟢 accepted
- **Date:** 2026-01-01
- **Deciders:** Fixture Owner
- **Applies to:** the development environment
- **Supersedes:** [ADR-0004](0004-dev-container-runtime.md)

## Context

The project is a small tool, and its one maintainer works on the host.

## Decision

We will work on the host, without a Dev Container.

**Out of scope:** nothing beyond the sentence above.

## Alternatives considered

None — cheap to reverse.

## Sources / Prior art

None — cheap to reverse.

## Consequences

None — cheap to reverse.

## Enforcement

**Not mechanically decidable:** where a maintainer works is outside the tree.
ADR
  edit "$d" docs/adr/0004-dev-container-runtime.md \
    "s/^- \*\*Status:\*\* 🟢 accepted\$/- **Status:** ⚪ superseded by $id/"
  # The index: the new row among the binding ones, the old row under the superseded ones.
  row="$(grep -F '| [0004](0004-dev-container-runtime.md) |' "$d/docs/adr/README.md" | sed 's/🟢 accepted/⚪ superseded/')"
  awk -v row="$row" -v new="| [$(printf '%04d' "$((n + 1))")]($file) | The work runs on the host | the development environment | 🟢 accepted |" '
    index($0, "| [0004](0004-dev-container-runtime.md) |") == 1 { next }
    /^### Superseded and rejected/ { print new; print ""; print; superseded = 1; next }
    superseded && $0 == "None yet." { print "| ADR | Title | Applies to | Status |"; print "|-----|-------|------------|--------|"; print row; next }
    { print }' "$d/docs/adr/README.md" > "$d/docs/adr/README.md.new" && mv "$d/docs/adr/README.md.new" "$d/docs/adr/README.md"
  # What the container brings, removed as the small-project path lists it.
  rm -r "$d/.devcontainer" "$d/scripts/check-devcontainer.sh" "$d/scripts/test-check-devcontainer.sh"
  awk '/^  devcontainer:$/ { skip = 1; next } skip && /^  [a-z][a-z0-9-]*:$/ { skip = 0 } !skip' \
    "$d/.github/workflows/checks.yml" > "$d/checks.yml.new" && mv "$d/checks.yml.new" "$d/.github/workflows/checks.yml"
  edit "$d" scripts/check-all.sh '/scripts\/\(test-\)\?check-devcontainer\.sh$/d'
  # shellcheck disable=SC2016  # the backticks are Markdown, not a command substitution
  edit "$d" README.md 's/`devcontainer`, //'
  edit "$d" .vscode/settings.json '/^  \/\/ Manage the \*host/,/^  }$/d'
  edit "$d" .gitignore '/^# Dev Container:/,/(ADR-0004)\.$/d'
  edit "$d" README.md '/^## Dev Container$/,/^## Coding Agents$/{/^## Coding Agents$/!d}'
  edit "$d" README.md '/^\.devcontainer\/ /d'
  edit "$d" README.md '/^This Dev Container preinstalls/,/^$/d'
  edit "$d" SECURITY.md '/^- \*\*The Dev Container has no access/,/^- \*\*Nothing leaves/{/^- \*\*Nothing leaves/!d}'
  stage "$d"
  expect_pass "the small-project path passes the documentation checks" "$d" check-docs.sh
  expect_pass "the small-project path passes the traceability check" "$d" check-traceability.sh
fi

# --- summary ---------------------------------------------------------------------------------

if ((failed)); then
  echo "Template setup self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Template setup self-test passed ($passed cases)."
exit 0
