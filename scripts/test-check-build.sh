#!/usr/bin/env bash
#
# Self-test of the dependency look of scripts/check-build.sh: where the web interface's
# dependencies are not installed as bun.lock locks them, the gate names the command that installs
# them and runs none of the web interface's commands, while the Rust commands run on.
#
# Each case builds a fixture under a temporary directory: the script, a bun.lock in the form bun
# writes it, with a root workspace and src-web, and the developer configuration the Rust tests
# read. node_modules/ holds what `bun install` puts there, each package's package.json at the
# version a case gives. cargo, bun, and rustup are stand-ins on PATH that record their arguments
# and pass; the Windows lint finds no cargo-xwin and is skipped. A case asserts the exit code, a
# needle in the output, and which commands ran.
#
# Inputs: scripts/check-build.sh
#
# Usage:
#     scripts/test-check-build.sh
# Exit code 0 when every case passes, 1 otherwise.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failed=0
passed=0

stubs="$work/stubs"
mkdir -p "$stubs"
for tool in cargo bun rustup; do
  # shellcheck disable=SC2016 # expanded by the stand-in when it runs
  printf '#!/usr/bin/env bash\necho "%s $*" >> "$STUB_LOG"\n' "$tool" > "$stubs/$tool"
  chmod +x "$stubs/$tool"
done

# fixture <name> — a repository whose dependencies are not installed.
fixture() {
  local dir="$work/$1"
  mkdir -p "$dir/scripts" "$dir/src-web" "$dir/crates/git/tests/developer-config/git"
  cp "$ROOT/scripts/check-build.sh" "$dir/scripts/"
  : > "$dir/crates/git/tests/developer-config/git/config"
  cat > "$dir/bun.lock" <<'LOCK'
{
  "lockfileVersion": 1,
  "configVersion": 1,
  "workspaces": {
    "": {
      "name": "ocelli",
      "devDependencies": {
        "@tauri-apps/cli": "^2.12.1",
      },
    },
    "src-web": {
      "name": "ocelli-web",
      "dependencies": {
        "@codemirror/state": "^6",
      },
      "devDependencies": {
        "globals": "^17.13.0",
        "vite": "^8",
      },
      "optionalDependencies": {
        "fsevents": "~2.3.3",
      },
      "peerDependencies": {
        "typescript": "^6",
      },
      "optionalPeers": [
        "typescript",
      ],
    },
  },
  "packages": {
    "@codemirror/state": ["@codemirror/state@6.6.0", "", {}, "sha512-x=="],

    "@tauri-apps/cli": ["@tauri-apps/cli@2.12.1", "", { "bin": { "tauri": "tauri.js" } }, "sha512-x=="],

    "fsevents": ["fsevents@2.3.3", "", {}, "sha512-x=="],

    "globals": ["globals@14.0.0", "", {}, "sha512-x=="],

    "typescript": ["typescript@6.0.2", "", {}, "sha512-x=="],

    "vite": ["vite@8.1.5", "", { "bin": { "vite": "bin/vite.js" } }, "sha512-x=="],

    "ocelli-web/globals": ["globals@17.13.0", "", {}, "sha512-x=="],
  }
}
LOCK
  echo "$dir"
}

# installed <dir> <workspace dir or ''> <name> <version> — put a package where bun installs it.
installed() {
  local at="$1/${2:+$2/}node_modules/$3"
  mkdir -p "$at"
  printf '{\n  "name": "%s",\n  "version": "%s"\n}\n' "$3" "$4" > "$at/package.json"
}

# every <dir> — every direct dependency installed at the version bun.lock locks.
every() {
  installed "$1" "" "@tauri-apps/cli" 2.12.1
  installed "$1" src-web "@codemirror/state" 6.6.0
  installed "$1" src-web globals 17.13.0
  installed "$1" src-web vite 8.1.5
}

RUST=("cargo build --workspace --locked" "cargo test --workspace --locked" "cargo fmt --all --check"
  "cargo clippy --workspace --all-targets --locked -- -D warnings")
WEB=("bun run --cwd src-web build" "bun test src-web" "bun run --cwd src-web lint")

# expect <pass|fail> <name> <dir> <needle> — run the script, assert its verdict and needle, that
# every Rust command ran, and that the web interface's commands ran exactly on a pass.
expect() {
  local want="$1" name="$2" dir="$3" needle="$4"
  local out rc log="$dir/stub.log" problem="" c
  : > "$log"
  out="$(cd "$dir" && STUB_LOG="$log" PATH="$stubs:/usr/bin:/bin" bash scripts/check-build.sh 2>&1)"
  rc=$?
  if [[ "$want" == "pass" ]]; then ((rc == 0)) || problem="exit $rc"; else ((rc != 0)) || problem="exit 0"; fi
  [[ "$out" == *"$needle"* ]] || problem+=" no \"$needle\""
  [[ "$out" == *"command not found"* ]] && problem+=" a command not found"
  for c in "${RUST[@]}"; do grep -qxF "$c" "$log" || problem+=" '$c' not run"; done
  for c in "${WEB[@]}"; do
    if [[ "$want" == "pass" ]]; then
      grep -qxF "$c" "$log" || problem+=" '$c' not run"
    else
      grep -qxF "$c" "$log" && problem+=" '$c' run"
    fi
  done
  if [[ -z "$problem" ]]; then
    passed=$((passed + 1))
    return
  fi
  failed=$((failed + 1))
  echo "FAIL: $name (expected $want:$problem)" >&2
  while IFS= read -r l; do printf '    %s\n' "$l"; done <<< "$out" >&2
}

d="$(fixture nothing)"
expect fail "no node_modules/ names the install command" "$d" \
  "(node_modules/@tauri-apps/cli is missing): run 'bun install --frozen-lockfile'."

d="$(fixture every)"
every "$d"
expect pass "every direct dependency at its locked version runs every command" "$d" \
  "Build, test and lint passed."

d="$(fixture workspace-missing)"
every "$d"
rm -r "$d/src-web/node_modules/vite"
expect fail "a dependency of src-web missing, the root's there" "$d" \
  "(src-web/node_modules/vite is missing)"

d="$(fixture stale)"
every "$d"
installed "$d" src-web vite 8.1.4
expect fail "a dependency at another version than bun.lock locks" "$d" \
  "(src-web/node_modules/vite is 8.1.4, bun.lock locks 8.1.5)"

d="$(fixture nested)"
every "$d"
installed "$d" src-web globals 14.0.0
expect fail "a dependency bun.lock nests under the workspace is held to that version" "$d" \
  "(src-web/node_modules/globals is 14.0.0, bun.lock locks 17.13.0)"

d="$(fixture no-version)"
every "$d"
printf '{\n  "name": "vite"\n}\n' > "$d/src-web/node_modules/vite/package.json"
expect fail "a package.json without a version" "$d" "(src-web/node_modules/vite has no version)"

d="$(fixture hoisted)"
every "$d"
rm -r "$d/src-web/node_modules/vite"
installed "$d" "" vite 8.1.5
expect pass "a dependency of src-web the hoisted linker put in the root's node_modules/" "$d" \
  "Build, test and lint passed."

d="$(fixture unlocked)"
every "$d"
sed '/"vite": \["vite@/d' "$d/bun.lock" > "$d/bun.lock.tmp" && mv "$d/bun.lock.tmp" "$d/bun.lock"
expect fail "a dependency bun.lock has no package for" "$d" \
  "(src-web/node_modules/vite is 8.1.5, bun.lock locks none)"

d="$(fixture unreadable)"
every "$d"
printf '{\n  "lockfileVersion": 9\n}\n' > "$d/bun.lock"
expect fail "a bun.lock that names no direct dependency this reads" "$d" \
  "(bun.lock names no direct dependency this gate can read)"

d="$(fixture no-lock)"
every "$d"
rm "$d/bun.lock"
expect fail "no bun.lock" "$d" "(bun.lock names no direct dependency this gate can read)"

if ((failed)); then
  echo "Build gate self-test FAILED ($failed of $((passed + failed)) cases)." >&2
  exit 1
fi
echo "Build gate self-test passed ($passed cases)."
