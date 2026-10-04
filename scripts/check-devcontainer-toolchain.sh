#!/usr/bin/env bash
#
# Checks that the Dev Container carries the toolchain ADR-0011 decides:
#   1. devcontainer.json builds from .devcontainer/Dockerfile instead of naming an image;
#   2. the Dockerfile starts from mcr.microsoft.com/devcontainers/base:debian and installs the
#      desktop host's Linux build library WebKitGTK (libwebkit2gtk-4.1-dev);
#   3. devcontainer.json adds the Rust and the Bun Features (ADR-0008); that they are pinned and
#      locked is scripts/check-devcontainer.sh's to check (ADR-0004);
#   4. rust-toolchain.toml pins a channel.
# Comment lines ('//' in the JSON, '#' in the Dockerfile) are ignored.
#
# Usage:
#     scripts/check-devcontainer-toolchain.sh [ROOT]   (ROOT defaults to the repository)
# Exit code 0 when the toolchain is there, 1 otherwise.

set -uo pipefail

ROOT="$(cd "${1:-$(dirname "${BASH_SOURCE[0]}")/..}" && pwd)"
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
