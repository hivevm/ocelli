# ADR-0011: The Dev Container is built from a Dockerfile that adds the desktop host's system libraries, with Rust and Bun as pinned Features

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `.devcontainer/`

## Context

The template ships the Dev Container as a base image with Features and no toolchain. The
toolchain (ADR-0008) needs Rust and Bun, and the desktop host builds on Linux against system
libraries no Feature provides: WebKitGTK and GTK, among others. The earlier code base on `main`
installed them through a Dockerfile on the same base image.

## Decision

We will build the Dev Container from a **Dockerfile** on
`mcr.microsoft.com/devcontainers/base:debian` that installs the desktop host's Linux build
libraries, and add Rust and Bun as Features pinned by major version tag and locked in
`devcontainer-lock.json`, the exact versions held by `rust-toolchain.toml` and the Bun Feature's
`version` option.

**Out of scope:** which libraries the desktop host needs, which follows ADR-0018; running the
desktop application's window from inside the container, which needs display forwarding from the
host; and macOS and Windows, which the build job covers.

## Alternatives considered

- **The base image with Features only** — no Dockerfile to maintain, but the desktop host would
  build only in CI.
- **A Compose file** — for services beside the workspace; Ocelli has none.

## Sources / Prior art

- Dev Containers, *Using Images, Dockerfiles, and Docker Compose* —
  <https://containers.dev/guide/dockerfile>.
- Tauri, *Prerequisites* — <https://v2.tauri.app/start/prerequisites/>: the Linux build libraries.

## Consequences

- Positive: the whole toolchain builds in the container, the desktop application included.
- Negative / trade-offs: a Dockerfile's packages are not locked like Features; a rebuild may pick
  up newer Debian packages.
- Follow-ups: the exact library list, with ADR-0018.

## Enforcement

`scripts/check-devcontainer-toolchain.sh` (job `build` of `checks.yml`) fails when
`devcontainer.json` names an image instead of building the Dockerfile, when the Dockerfile starts
from another base or does not install WebKitGTK, when the Rust or the Bun Feature is missing, or
when `rust-toolchain.toml` pins no channel; its self-test cites this ADR with a `Verifies:` marker.
`scripts/check-devcontainer.sh` verifies that every Feature is pinned and locked and that no host
socket is mounted (ADR-0004).
