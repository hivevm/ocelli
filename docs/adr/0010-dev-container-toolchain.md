# ADR-0010: The Dev Container is built from a Dockerfile with Rust and Bun as Features, and runs no container engine

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `.devcontainer/Dockerfile`, the Features and the image of `.devcontainer/devcontainer.json`, `scripts/check-devcontainer-toolchain.sh`, and the tests that need a service or a container

## Context

[ADR-0004](0004-dev-container-runtime.md) keeps the host's container engine out of the Dev
Container's reach, locks its Features, and mounts each project under a path of its own. It leaves
to the project which base image and Features the container uses, and whether an engine runs
inside it.

Ocelli's toolchain ([ADR-0008](0008-toolchain-rust-and-typescript.md)) needs Rust and Bun, and
the desktop host ([ADR-0027](0027-tauri-desktop-host.md)) builds on Linux against system
libraries no Feature provides: WebKitGTK and GTK, among others. Ocelli's tests need Git
repositories, which they create in temporary directories, and a remote server, which runs as a
process; no test needs a database, a service, or a container.

## Decision

We will build the Dev Container from a Dockerfile that installs the desktop host's Linux build
libraries. Rust and Bun come as Features, pinned as
[ADR-0004](0004-dev-container-runtime.md) pins every Feature. No container engine runs inside it.

- **The image.** `.devcontainer/Dockerfile` starts from
  `mcr.microsoft.com/devcontainers/base:debian` and installs the desktop host's Linux build
  libraries; `devcontainer.json` builds it rather than naming an image.
- **The Features.** Rust and Bun are added as Features. The exact versions are held by
  `rust-toolchain.toml` and the Bun Feature's `version` option.
- **No engine inside.** The container runs no container engine and holds no socket of one.

**Out of scope:** which libraries the desktop host needs, which follows
[ADR-0027](0027-tauri-desktop-host.md). Which extensions the project picks, and whether it pins
them. Running the desktop application's window from inside the container, which needs display
forwarding from the host. macOS and Windows, which the build job covers.

## Alternatives considered

- **An engine inside the container, or a Compose file** — for services beside the workspace, but
  no test needs one; either is added by superseding this ADR once a test does.
- **The base image with Features only** — no Dockerfile to maintain, but the desktop host would
  build only in CI.

## Sources / Prior art

- Dev Containers, *Using Images, Dockerfiles, and Docker Compose* —
  <https://containers.dev/guide/dockerfile>.
- Tauri, *Prerequisites* — <https://v2.tauri.app/start/prerequisites/>: the Linux build libraries.

## Consequences

- Positive: the whole toolchain builds in the container, the desktop application included.
- Negative / trade-offs: a Dockerfile's packages are not locked like Features, so a rebuild may
  pick up newer Debian packages.
- Follow-ups: the exact library list, with [ADR-0027](0027-tauri-desktop-host.md).

## Enforcement

[`scripts/check-devcontainer-toolchain.sh`](../../scripts/check-devcontainer-toolchain.sh) (job
`build` of [`checks.yml`](../../.github/workflows/checks.yml)) fails in four cases. One:
[`devcontainer.json`](../../.devcontainer/devcontainer.json) names an image instead of building
the Dockerfile. Two: the Dockerfile starts from another base or does not install WebKitGTK.
Three: the Rust or the Bun Feature is missing. Four: `rust-toolchain.toml` pins no channel. Its
self-test cites this ADR with a `Verifies:` marker
([ADR-0002](0002-decisions-verified-by-tests.md)). Not checked: the container itself — no CI job
builds it.

**Not mechanically decidable:** that no engine runs inside, because a Feature or a package can
add one under any name; review reads every change to `.devcontainer/`.
