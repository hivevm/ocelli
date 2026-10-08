#!/usr/bin/env bash
#
# Runs cargo-mutants over the git adapter's guards, the checks every write makes before it changes
# anything (crates/git/src/guard.rs) and the checks of a remote's address and its credentials
# (crates/git/src/remote.rs): each mutant changes one guard, and the suite of the git adapter must
# fail on it. A mutant the suite lets through is a guard no test decides, as the credential guard
# against line breaks was until its own test came (ADR-0033, ADR-0035). The surviving mutants are
# listed, and appended to the job summary when GITHUB_STEP_SUMMARY is set.
#
# Usage:
#     scripts/measure-mutants.sh
# Environment, optional: OCELLI_MUTANTS_JOBS, the mutants built and tested at once (2).
# Exit code 0 when every mutant is caught, 1 when one survives or the run cannot be made.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="27.1.0"
jobs="${OCELLI_MUTANTS_JOBS:-2}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

if ! cargo mutants --version 2>/dev/null | grep -qx "cargo-mutants $VERSION"; then
  cargo install cargo-mutants --locked --version "$VERSION" || {
    echo "Mutants: cargo-mutants $VERSION cannot be installed." >&2
    exit 1
  }
fi

# Mutants no test can decide, each for its reason: which words a failed transfer's refusal takes,
# not whether it is refused; the server timeouts, the known_hosts files, and the local user name,
# which a test would change for the whole process; and changes that leave a function as it was,
# a bit or-ed into zeros, a port that never reaches the check because the address does not parse,
# and a comment or a marker line whose first field is never a host.
exclude=(
  --exclude-re 'Transfer.*::refusal'
  --exclude-re 'set_server_timeouts|known_hosts_files|local_user'
  --exclude-re 'replace \| with \^ in decode_base64'
  --exclude-re 'match guard port\.chars\(\)\.all.* in helper_may_answer_for'
  --exclude-re 'remote\.rs:[0-9]+:61: replace && with \|\| in vouched_elsewhere'
)

cd "$ROOT" || exit 1
cargo mutants --package ocelli-git --file crates/git/src/guard.rs --file crates/git/src/remote.rs \
  "${exclude[@]}" --jobs "$jobs" --output "$out"
status=$?
missed="$out/mutants.out/missed.txt"
if [[ -s "$missed" ]]; then
  echo "Mutants the suite lets through:"
  cat "$missed"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
      echo "### Mutants the suite lets through"
      echo '```'
      cat "$missed"
      echo '```'
    } >>"$GITHUB_STEP_SUMMARY"
  fi
fi
if ((status != 0)); then
  echo "Mutants: the suite lets a guard change go unnoticed, or the run failed (exit $status)." >&2
  exit 1
fi
echo "Mutants: every guard change fails the suite."
