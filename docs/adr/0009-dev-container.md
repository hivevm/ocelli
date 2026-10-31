# ADR-0009: The Dev Container is built from a Dockerfile with locked Features, and keeps every container engine out of reach

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** NUC maintainer, Markus Brigl
- **Applies to:** `.devcontainer/`, `.vscode/settings.json`, `scripts/check-devcontainer.sh`, `scripts/check-devcontainer-toolchain.sh`, the tests that need a service or a container, and everything the container pulls in

## Context

The template ships a Dev Container so that the first run is reproducible without committing to a
language toolchain. Ocelli's toolchain ([ADR-0007](0007-toolchain-rust-and-typescript.md)) needs
Rust and Bun, and the desktop host ([ADR-0021](0021-tauri-desktop-host.md)) builds on Linux
against system libraries no Feature provides: WebKitGTK and GTK, among others.

Agents often want Docker access, and the obvious way, the `docker-outside-of-docker` Feature,
bind-mounts the **host** Docker socket, handing host-level control to anything inside. Managing
host containers from VS Code does not need the socket. Ocelli's tests need Git repositories,
which they create in temporary directories, and a remote server, which runs as a process; no
test needs a database, a service, or a container.

The container also pulls in third-party code that runs next to the agent credentials it mounts:
**Features**, executed at build time. A Feature tag is mutable and can be repointed, so a tag
alone does not say what a commit builds.

## Decision

We will build the Dev Container from a Dockerfile with Rust and Bun as Features locked by
`devcontainer-lock.json`, and keep every container engine out of its reach, the host's and one
of its own.

- **The image.** `.devcontainer/Dockerfile` starts from
  `mcr.microsoft.com/devcontainers/base:debian` and installs the desktop host's Linux build
  libraries.
- **The Features.** Rust and Bun are added as Features, each pinned by major version tag, with
  `.devcontainer/devcontainer-lock.json` committed, so the same commit yields the same Features.
  The exact versions are held by `rust-toolchain.toml` and the Bun Feature's `version` option.
- **No host engine.** No host Docker socket is mounted and no Feature that would mount one is
  added, in every variant. Host containers are managed from an extension pinned to the host side
  via `remote.extensionKind` in [`.vscode/settings.json`](../../.vscode/settings.json).
- **No engine inside.** The container runs no container engine and holds no socket of one.
- **The git hooks.** The container's `postStartCommand` enables the repository's git hooks with
  the merge settings they rely on, its share of the local gate of
  [ADR-0002](0002-decisions-verified-by-tests.md).

**Out of scope:** which libraries the desktop host needs, which follows
[ADR-0021](0021-tauri-desktop-host.md); which extensions a project picks, and whether it pins
them; whether work may happen outside the container; hardening the container, which is not a
security boundary; how the lock is refreshed; running the desktop application's window from
inside the container, which needs display forwarding from the host; and macOS and Windows,
which the build job covers.

## Alternatives considered

- **`docker-outside-of-docker`** — mounts the host socket and hands host-level control to
  everything inside, the agent included.
- **An engine inside the container, or a Compose file** — for services beside the workspace, but
  no test needs one; either is added by superseding this ADR once a test does.
- **The base image with Features only** — no Dockerfile to maintain, but the desktop host would
  build only in CI.
- **Ignore the Feature lock** — a repointed tag reaches the credentials before anyone sees a diff.
- **Digests directly in `devcontainer.json`** — buries hashes in the hand-edited file and loses the
  readable tag.

## Sources / Prior art

- Dev Container specification — <https://containers.dev/implementors/json_reference/> and
  <https://containers.dev/features>.
- Dev Containers, *Using Images, Dockerfiles, and Docker Compose* —
  <https://containers.dev/guide/dockerfile>.
- `remote.extensionKind` — <https://code.visualstudio.com/docs/devcontainers/containers>.
- Docker daemon attack surface —
  <https://docs.docker.com/engine/security/#docker-daemon-attack-surface>.
- Tauri, *Prerequisites* — <https://v2.tauri.app/start/prerequisites/>: the Linux build libraries.

## Consequences

- Positive: the whole toolchain builds in the container, the desktop application included; the
  security posture is the same in every variant; the same commit gives the same Features, and a
  changed digest shows up in review.
- Negative / trade-offs: Feature updates stop arriving on their own and a stale lock is
  invisible; a Dockerfile's packages are not locked like Features, so a rebuild may pick up newer
  Debian packages.
- Follow-ups: whether an automated update path for Features should be wired up; the exact
  library list, with [ADR-0021](0021-tauri-desktop-host.md).

## Enforcement

[`scripts/check-devcontainer.sh`](../../scripts/check-devcontainer.sh) (job `devcontainer` in
[`checks.yml`](../../.github/workflows/checks.yml)) reads the files this decision names and
fails when [`devcontainer.json`](../../.devcontainer/devcontainer.json) mounts a host socket or
adds a Feature that would, when a Feature carries no major version tag, when the lock is missing
or names other Features than the definition, or when
[`.vscode/settings.json`](../../.vscode/settings.json) no longer pins the container-management
extension to the host side, or when the container's `postStartCommand` no longer enables the
repository's git hooks with the merge settings they rely on (`core.hooksPath`, `merge.ff`,
`pull.ff`, `merge.adr-index.driver`).
[`scripts/check-devcontainer-toolchain.sh`](../../scripts/check-devcontainer-toolchain.sh) (job
`build` of `checks.yml`) fails when `devcontainer.json` names an image instead of building the
Dockerfile, when the Dockerfile starts from another base or does not install WebKitGTK, when the
Rust or the Bun Feature is missing, or when `rust-toolchain.toml` pins no channel. The self-test
of each cites this ADR with a `Verifies:` marker
([ADR-0002](0002-decisions-verified-by-tests.md)). Not checked: a variant definition under a
subdirectory of `.devcontainer/`, whether the lock's digests are the ones the tags currently
resolve to, and the container itself — no CI job builds it.

**Not mechanically decidable:** that no engine runs inside, because a Feature or a package could
add one under any name; review reads every change to `.devcontainer/`.
