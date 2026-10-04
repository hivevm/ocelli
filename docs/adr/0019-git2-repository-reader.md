# ADR-0019: git2 reads repositories behind the repository port, off the thread that draws

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `crates/git/` (package `ocelli-git`), the `RepositoryReader` port in `crates/core/src/port.rs`, and every call of it from a front end

## Context

`core` reaches a repository only through its ports ([ADR-0009](0009-ports-and-adapters.md)); the
reading port is the trait `RepositoryReader` in `crates/core/src/port.rs`, and something has to
implement it. Loading a repository is I/O-bound and takes noticeable time, chunk after chunk over
the whole history ([ADR-0016](0016-history-loads-in-chunks.md)), and G-13 requires that the front
end keeps responding and reports progress while it runs. Q-4 requires that showing a repository
starts no program the repository names, such as `core.fsmonitor`, a hook, or a filter driver.

The Git library is therefore chosen together with the way a front end calls it: the desktop host
runs on Tokio, which Tauri requires ([ADR-0018](0018-tauri-desktop-host.md)), and the terminal
application runs a plain event loop of its own.

## Decision

We will implement `RepositoryReader` in `ocelli-git` with **`git2`** (the bindings to libgit2),
reporting progress through a callback the port takes, and have every front end call it off the
thread that draws: the desktop host on Tokio's blocking pool through `tokio::task::spawn_blocking`,
the terminal application on a `std::thread` of its own.

**Out of scope:** the operations that write (G-4 to G-7), which may reach a repository through
other means behind ports of their own, which is [ADR-0033](0033-writes-what-git-writes.md); the
remote server's threading.

## Alternatives considered

- **`gitoxide`** — pure Rust, no C dependency, but less mature and less documented for the ref
  walking and diffing Ocelli needs.
- **Calling the `git` executable and parsing its output** — no library at all, but it ties reading
  to an external binary and to the stability of its output, and `git status` starts
  `core.fsmonitor` and other configured programs, which Q-4 forbids unless every call overrides
  them.
- **Calling `git2` directly in the async task or the event loop** — least code, but it blocks a
  runtime worker or the drawing thread for the whole load, the failure G-13 rules out.
- **A Tokio channel in the port for progress** — the shape of the desktop host's runtime, but it
  pulls Tokio into `core`, which ADR-0009 keeps free of technology; a callback serves both hosts.

## Sources / Prior art

- `git2` and libgit2 — <https://docs.rs/git2> and <https://libgit2.org/>
- `gitoxide` — <https://github.com/Byron/gitoxide>
- Tokio, `spawn_blocking` and the guidance on blocking work —
  <https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html>
- `git config`, `core.fsmonitor` —
  <https://git-scm.com/docs/git-config#Documentation/git-config.txt-corefsmonitor>
- GitGraph's `docs/adr/0014-git2-on-spawn-blocking.md` on `main`, where this reader runs.

## Consequences

- Positive: `git2` is proven and covers everything the viewer reads; the front ends stay
  responsive while a repository loads (G-13); `core` stays free of Tokio; libgit2 starts no hook,
  external filter driver, or file-system monitor, which is the ground Q-4 stands on.
- Negative / trade-offs: a blocking task costs a pool slot, negligible for a load; the progress
  callback runs on the loading thread, so each host carries it to its drawing thread itself;
  linking libgit2 makes cross-compiling harder than a pure-Rust library would; that libgit2 starts
  nothing is a property of a C library Ocelli does not control and holds only while a test says so.
- Follow-ups: `gitoxide`, if it matures enough to drop the C dependency; the means of the writing
  operations.

## Enforcement

A test in `crates/git/tests/` cites this ADR and Q-4 with a `Verifies:` marker: it opens a fixture
repository whose `core.fsmonitor` and hooks name a program that would leave a marker file, loads
it through `RepositoryReader`, and asserts that no marker appears.

**Not mechanically decidable:** that no `git2` call runs on the thread that draws. Observing the
thread a call ran on needs a harness around each host's runtime, and the failure is a stalled window
rather than a wrong value. The call sites are few, in `crates/commands/src/lib.rs`, through which
the desktop host calls the port, and the terminal application's loader, so this is review: every
call of the port there sits inside `spawn_blocking` or on the loader's thread.
