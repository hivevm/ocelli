#!/usr/bin/env bash
#
# Runs cargo-mutants over the git adapter's guards, the checks every write makes before it changes
# anything (crates/git/src/guard.rs), the checks of a remote's address and its credentials
# (crates/git/src/remote.rs), the confinement to a server root (crates/git/src/confine.rs), and
# the two reads the remote server's turns key and compare a repository by (common_dir and
# refs_digest in crates/git/src/refs.rs), which the remote server's tests decide too, so the web
# interface must be built first: each mutant changes one guard, and the suites of the git adapter
# and the server must fail on it. A mutant the suite lets through is a guard no test decides
# (ADR-0007). The surviving mutants are
# listed, and appended to the job summary when GITHUB_STEP_SUMMARY is set.
#
# Usage:
#     scripts/measure-mutants.sh
# Environment, optional: OCELLI_MUTANTS_JOBS, the mutants built and tested at once (2).
# Exit code 0 when every mutant is caught, 1 when one survives or the run cannot be made.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# A measurement takes minutes, and scripts/check-all.sh runs every script a workflow runs, so it
# skips itself unless OCELLI_MEASURE_ALL=1, which .github/workflows/measure.yml sets.
if [[ "${OCELLI_MEASURE_ALL:-}" != 1 ]]; then
  echo "Mutants of the git adapter's guards: skipped — measured weekly by .github/workflows/measure.yml; OCELLI_MEASURE_ALL=1 measures here."
  exit 0
fi
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
# and a comment or a marker line whose first field is never a host. The other reads of refs.rs,
# beside the turns' two, are no guard: their tests decide what they read, not what they let in.
exclude=(
  --exclude-re 'Transfer.*::refusal'
  --exclude-re 'set_server_timeouts|known_hosts_files|local_user'
  --exclude-re 'replace \| with \^ in decode_base64'
  --exclude-re 'match guard port\.chars\(\)\.all.* in helper_may_answer_for'
  --exclude-re 'remote\.rs:[0-9]+:61: replace && with \|\| in vouched_elsewhere'
  --exclude-re 'refs\.rs:.* (replace|in) (load_[a-z_]+|default_remote|short|remote_names|split_remote_branch|merged|head_of|main_repo|dir_name|same_dir|submodule_state|find_parent|operation_of|stopped|conflicts_of)\b'
)

cd "$ROOT" || exit 1
cargo mutants --package ocelli-git --file crates/git/src/guard.rs --file crates/git/src/remote.rs \
  --file crates/git/src/confine.rs --file crates/git/src/refs.rs \
  --test-package ocelli-git --test-package ocelli-server \
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
