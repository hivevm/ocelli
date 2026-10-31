# ADR-0038: A pushed version tag builds the installers and a Flatpak into a draft release

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `.github/workflows/release.yml` and `flatpak/`

## Context

Ocelli is free software for a broad audience on Linux, macOS, and Windows (Mission, Constraints).
A developer has to be able to download it: without automation every release means building
installers for three operating systems by hand, on three machines, which one maintainer
(Constraints) does not do reliably. On Linux, a Flatpak runs across distributions without
packaging for each.

Distribution independence is Flatpak's core property: one bundle, built against a fixed runtime,
runs on any distribution with Flatpak installed, so there is nothing to build per distribution.
Tauri ([ADR-0021](0021-tauri-desktop-host.md)) already bundles the distribution-native `.deb`,
`.rpm`, and AppImage beside it.

The pipeline runs third-party actions with a token that can write releases, so every action is
referenced by its full commit SHA and kept current by Dependabot
([ADR-0010](0010-action-references-sha.md)). Building runs the toolchain of
[ADR-0007](0007-toolchain-rust-and-typescript.md), Bun and cargo through the Tauri CLI. Releasing
stays a human's act.

## Decision

We will add a GitHub Actions workflow, triggered by pushing a `v*` tag, that builds every
installer and attaches it to a **draft** GitHub release named after the version, which the
maintainer reviews and publishes.

1. **Platform installers through `tauri-apps/tauri-action`**, in a matrix: macOS on
   `macos-latest` for `aarch64-apple-darwin` and `x86_64-apple-darwin` (`.dmg`, `.app`), Windows on
   `windows-latest` (`.msi` and an NSIS `.exe`), Linux on `ubuntu-22.04` (`.deb`, `.rpm`,
   AppImage). The action runs `tauri build`, takes the version from the workspace's `Cargo.toml`,
   and uploads every bundle to the draft release.
2. **One Flatpak bundle through `flatpak/flatpak-github-actions`**, from a manifest in `flatpak/`
   named after the application's identifier in `tauri.conf.json`, following Tauri's Flatpak guide:
   the GNOME 46 runtime and SDK, a module that installs the `.deb` of step 1, and `finish-args`
   limited to what the application needs (the Wayland and X11 sockets, `dri`, `ipc`, the host's
   files to read and write repositories, and the network for remotes and avatars). The job
   attaches `Ocelli-<version>.flatpak` to the same draft release.
3. **Unsigned binaries for now.** Code signing for Windows and signing and notarisation for macOS
   need certificates the project does not have; developers see the usual warnings of the
   operating system.
4. **Every action by its full commit SHA**, with the version as a comment, as ADR-0010 decides.

**Out of scope:** code signing and notarisation; Flathub; an updater inside the application;
packaging the terminal application (`ocelli-cli`) and the remote server.

## Alternatives considered

- **Building by hand on the maintainer's machines** — three environments to keep, not
  reproducible, and in practice no downloads at all.
- **A build workflow of our own instead of `tauri-action`** — full control, but it re-implements
  what the official action does (system dependencies per platform, bundling, the upload, the
  version); more YAML for no capability.
- **Flathub instead of a bundled `.flatpak`** — discoverability and automatic updates, but an
  external review in a separate repository that ties releases to Flathub's timeline; it stays open.
- **Native packages per distribution (PPA, AUR, COPR)** — exactly the per-distribution burden
  Flatpak exists to avoid; the `.deb` and `.rpm` Tauri bundles already cover the two largest
  families.
- **Publishing at once instead of as a draft** — one click less, but the pipeline would become
  the one who releases, instead of the human who decides to.

## Sources / Prior art

- Tauri, *GitHub* pipelines: the `tauri-action` matrix with macOS split by architecture and draft
  releases — <https://v2.tauri.app/distribute/pipelines/github/>
- Tauri, *Flatpak*: the GNOME runtime, a manifest that installs the `.deb`, and the required
  `finish-args` — <https://v2.tauri.app/distribute/flatpak/>
- `tauri-apps/tauri-action` — <https://github.com/tauri-apps/tauri-action>
- `flatpak/flatpak-github-actions` — <https://github.com/flatpak/flatpak-github-actions>
- Flatpak, *Basic concepts*, runtimes as the mechanism of distribution independence —
  <https://docs.flatpak.org/en/latest/basic-concepts.html>
- GitGraph's `docs/adr/0022-tag-triggered-release-pipeline.md` on `main`, where this pipeline runs.

## Consequences

- Positive: pushing a tag yields a complete, versioned set of downloads (`.dmg` for two
  architectures, `.msi`, NSIS `.exe`, `.deb`, `.rpm`, AppImage, `.flatpak`) on a draft release the
  maintainer publishes; the build is reproducible on clean runners; the SHA pins make every change
  of the code the release job runs a reviewed diff.
- Negative / trade-offs: unsigned binaries trigger Gatekeeper and SmartScreen warnings; four
  runner jobs per release cost CI minutes, macOS runners the most; the Flatpak manifest is one more
  artefact to keep in step (runtime upgrades, new permissions); `tauri-action` and the Flatpak
  action become dependencies of the supply chain, held by SHA and updated by Dependabot.
- Follow-ups: code signing and notarisation once certificates exist; Flathub; an updater; how the
  terminal application and the remote server are packaged and released.

## Enforcement

`scripts/sensor-action-refs.sh --require sha` of ADR-0010 covers `release.yml` like every other
workflow and fails on an action not pinned by full SHA.

**Not mechanically decidable:** that a tag produces the installers this ADR lists. The proof is a
release run, which needs a tag push and runners on several platforms; a check could only restate
the workflow's own contents. The draft release is the gate: a human sees what was built before
publishing it.
