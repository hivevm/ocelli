# ADR-0041: On macOS, Ocelli builds OpenSSL into its binaries and links no library outside the system

- **Status:** 🟡 proposed
- **Date:** 2026-10-10
- **Deciders:** Markus Brigl
- **Applies to:** the OpenSSL that `crates/git/Cargo.toml` links on macOS, `Cargo.lock`, and the macOS linkage check of the `build` job in `.github/workflows/checks.yml`

## Context

[ADR-0035](0035-remotes-through-libgit2-transports.md) reaches remotes through libgit2's own
HTTPS and SSH transports, `git2` with its `https` and `ssh` features. On macOS libgit2 speaks
HTTPS through the system's SecureTransport and hashes through CommonCrypto, but SSH goes through
libssh2, and `libssh2-sys` uses OpenSSL as its cryptography on every platform but Windows.
`openssl-sys` finds that OpenSSL on the build machine and links it dynamically.

macOS ships no OpenSSL to link against. On the `macos-latest` runner that
[ADR-0025](0025-tag-triggered-release.md) builds both macOS installers on, the OpenSSL found is
Homebrew's, for arm64 only:

- The `aarch64-apple-darwin` application names `/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib`
  and `libcrypto.3.dylib` as libraries to load. On a Mac without Homebrew's `openssl@3` the
  loader does not find them, and the application cannot start. The same holds for `ocelli-cli`
  and `ocelli-server` built there.
- The `x86_64-apple-darwin` leg cross-compiles on that arm64 runner, finds no OpenSSL for x86_64,
  and fails to build.

The constraints of [`docs/SPECIFICATION.md`](../SPECIFICATION.md) ask that every front end run on
Linux, macOS, and Windows. Linux and Windows are not affected: a Linux package links the
distribution's OpenSSL, which the distribution keeps patched, and libssh2 uses Windows' own CNG.

## Decision

We will build OpenSSL from source into Ocelli's binaries on macOS, through the `vendored` feature
of `openssl-sys` declared for `target_os = "macos"` only, so that nothing Ocelli ships for macOS
links a library outside the system.

**Out of scope:** OpenSSL on Linux and Windows, which stays as it is; replacing libssh2 or OpenSSL
altogether, which would mean transports of Ocelli's own and is ADR-0035's to decide; how Ocelli
learns of a vulnerability in the OpenSSL it ships (see Follow-ups).

## Alternatives considered

- **An Intel runner for the `x86_64-apple-darwin` leg** — builds the x86_64 installer natively,
  but links Homebrew's OpenSSL there too, so neither application starts on a Mac without it; and
  it departs from ADR-0025, which builds both architectures on `macos-latest`.
- **Copying Homebrew's OpenSSL into the application bundle** and rewriting the load paths with
  `install_name_tool` — keeps a dynamic library, but needs both architectures of OpenSSL on the
  runner, a post-build step Tauri does not offer, and covers the application bundle only, not
  `ocelli-cli` and `ocelli-server`.
- **Vendored OpenSSL on Linux too** — one way on both Unix platforms, but a Linux package would
  stop receiving the distribution's OpenSSL fixes, and nothing on Linux is broken.
- **rustls in place of OpenSSL** — rustls is TLS, while libssh2 needs a cryptography library;
  neither `libssh2-sys` nor `libgit2-sys` offers it. Using it would mean HTTPS and SSH transports
  of Ocelli's own under `git2::transport::register`, which ADR-0035 rejects.
- **gitoxide (`gix`)** — speaks HTTPS through rustls, but SSH by starting the `ssh` program, which
  Q-4 forbids.
- **AWS-LC through `openssl-sys`'s `aws-lc` feature** — also built from source, but libssh2 is
  written against OpenSSL's interface, and whether it builds and passes against AWS-LC was not
  established.
- **Dropping SSH on macOS** — removes OpenSSL, but breaks the developer's SSH remotes the
  constraints and ADR-0035 require.

## Sources / Prior art

- `libssh2-sys` 0.3.3 `build.rs`: `LIBSSH2_OPENSSL` on every target but Windows, `LIBSSH2_WINCNG`
  there; features `vendored-openssl` and `openssl-on-win32` only.
- `libgit2-sys` 0.18.8 `build.rs`: `GIT_SECURE_TRANSPORT` and `GIT_SHA256_COMMON_CRYPTO` on Apple
  targets, `GIT_OPENSSL` on other Unix targets, `GIT_WINHTTP` on Windows.
- `openssl-sys` 0.9.117: feature `vendored` builds `openssl-src` (`^300.2`, OpenSSL 3.x) and links
  it statically — <https://docs.rs/openssl/latest/openssl/#vendored>.
- The load commands of `Ocelli.app/Contents/MacOS/ocelli` in the draft release `v0.1.0`
  (`Ocelli_0.1.0_aarch64.app.tar.gz`), and the failed `x86_64-apple-darwin` leg of the release
  run for `5587fcd`.

## Consequences

- Positive: the macOS application, `ocelli-cli`, and `ocelli-server` start on any supported Mac,
  and the `x86_64-apple-darwin` leg builds on `macos-latest` as ADR-0025 has it. SSH remotes keep
  working as ADR-0035 decides (Q-4, Q-7 unchanged).
- Negative / trade-offs: Ocelli ships its own OpenSSL on macOS, and a fix to OpenSSL reaches
  macOS users only with an Ocelli release that updates `openssl-src`. A macOS build compiles
  OpenSSL, which takes longer and needs `perl` and `make` on the build machine. OpenSSL 3 is under
  the Apache License 2.0, whose notice the macOS distributions carry.
- Follow-ups: a check that reports known vulnerabilities in the Rust dependencies, `openssl-src`
  among them, since none watches them today.

## Enforcement

A step of the `build` job on `macos-latest` in `.github/workflows/checks.yml`, citing this ADR
with a `Verifies:` marker, lists the libraries the built `ocelli`, `ocelli-cli`, and
`ocelli-server` load (`otool -L`) and fails on any outside `/usr/lib/` and `/System/Library/`.
