#!/usr/bin/env bash
#
# Checks that the Dev Container carries the toolchain DEV-container-from-dockerfile decides:
#   1. devcontainer.json builds from .devcontainer/Dockerfile instead of naming an image;
#   2. the Dockerfile starts from mcr.microsoft.com/devcontainers/base:debian and installs the
#      desktop host's Linux build library WebKitGTK (libwebkit2gtk-4.1-dev);
#   3. devcontainer.json adds the Rust and the Bun Features (ARCH-rust-core-typescript-web); that they are pinned and
#      locked is scripts/check-devcontainer.sh's to check (NUC-container-hides-host-daemon §3);
#   4. rust-toolchain.toml pins a channel.
# Comment lines ('//' in the JSON, '#' in the Dockerfile) are ignored.
#
# Usage:
#     scripts/check-devcontainer-toolchain.sh [--root <dir>]   (the root defaults to the repository)
# Exit code 0 when the toolchain is there, 1 otherwise.

set -uo pipefail

# shellcheck source-path=SCRIPTDIR source=lib/root.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/root.sh"
usage="scripts/check-devcontainer-toolchain.sh [--root <dir>]"
enter_root "${BASH_SOURCE[0]}" "$usage" "$@"
# The build job runs this script on macOS with the bash the system ships, 3.2, where an empty array
# expanded whole is unbound under set -u; the form below expands to nothing there.
rest_args "${BASH_SOURCE[0]}" "$usage" 0 ${ARGS[@]+"${ARGS[@]}"}
CONFIG="$ROOT/.devcontainer/devcontainer.json"
DOCKERFILE="$ROOT/.devcontainer/Dockerfile"
TOOLCHAIN="$ROOT/rust-toolchain.toml"

errors=()

if [[ ! -f "$CONFIG" ]]; then
  errors+=(".devcontainer/devcontainer.json: not found")
else
  config="$(grep -v '^[[:space:]]*//' "$CONFIG")"
  grep -Eq '"dockerfile"[[:space:]]*:[[:space:]]*"Dockerfile"' <<< "$config" \
    || errors+=(".devcontainer/devcontainer.json: does not build from the Dockerfile")
  grep -Eq '^[[:space:]]*"image"[[:space:]]*:' <<< "$config" \
    && errors+=(".devcontainer/devcontainer.json: names an image instead of building the Dockerfile")
  grep -q '"ghcr.io/devcontainers/features/rust:' <<< "$config" \
    || errors+=(".devcontainer/devcontainer.json: lacks the Rust Feature")
  grep -q '"ghcr.io/devcontainers-extra/features/bun:' <<< "$config" \
    || errors+=(".devcontainer/devcontainer.json: lacks the Bun Feature")
fi

if [[ ! -f "$DOCKERFILE" ]]; then
  errors+=(".devcontainer/Dockerfile: not found")
else
  dockerfile="$(grep -v '^[[:space:]]*#' "$DOCKERFILE")"
  first_from="$(grep -m1 -E '^[[:space:]]*FROM[[:space:]]' <<< "$dockerfile")"
  [[ "$first_from" == *"mcr.microsoft.com/devcontainers/base:debian"* ]] \
    || errors+=(".devcontainer/Dockerfile: does not start from mcr.microsoft.com/devcontainers/base:debian")
  grep -q 'libwebkit2gtk-4.1-dev' <<< "$dockerfile" \
    || errors+=(".devcontainer/Dockerfile: does not install libwebkit2gtk-4.1-dev")
fi

if [[ ! -f "$TOOLCHAIN" ]] || ! grep -Eq '^[[:space:]]*channel[[:space:]]*=' "$TOOLCHAIN"; then
  errors+=("rust-toolchain.toml: missing, or pins no channel")
fi

if ((${#errors[@]})); then
  echo "Dev Container toolchain check FAILED:" >&2
  printf '  - %s\n' "${errors[@]}" >&2
  exit 1
fi
echo "Dev Container toolchain check passed (Dockerfile, WebKitGTK, Rust and Bun Features, pinned Rust)."
