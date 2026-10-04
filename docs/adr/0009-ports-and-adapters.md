# ADR-0009: The code is structured as ports and adapters, with a core crate that depends on no technology

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** every crate of the Cargo workspace and the dependencies between them

## Context

Ocelli puts several technologies behind the same behaviour. Three front ends drive it: the
desktop application, the remote server with its web interface, and the terminal application
(G-8). On the other side, reading a history and rewriting it may want different means: a Git
library reads fast, while rebase, cherry-pick, and conflict resolution (G-5, G-6) are what `git`
itself does best. And Q-4 asks that showing a repository runs no program the repository names,
which is a property of how the repository is reached, and must be held at one place.

The earlier code base on `main` decided ports and adapters, and then kept the Git access and the
layout in one crate, `adapter-git`: the layout could be tested only against real repositories,
and the core and the technology were one thing in name and in code.

From the catalogue: ports and adapters fits where the domain outlives its technologies and must
be tested without them; it does not fit a thin CRUD application or a small CLI, which Ocelli is
not.

## Decision

We will structure the Rust code as **ports and adapters**: a crate `core` holds the commit graph,
its layout, and the operations on a repository, and owns the ports through which it reaches a
repository; every technology, the Git access and each front end, is an adapter in a crate of its
own that depends on `core`, and `core` depends on no adapter and on no technology.

**Out of scope:** the names of the adapter crates and the ports, which the golden path shows and
[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) lists; the internal structure of `core`, of which
the layout's is the next ADR's; and the web interface, which talks to an adapter and not to
`core`.

## Alternatives considered

- **One crate for Git access and layout, as on `main`** — simpler by one crate, but the layout is
  tested only through real repositories and Q-4's guarantee has no single place.
- **Layered** — familiar, but the direction runs top-down, so the Git access shapes the core.
- **Clean or onion architecture** — the same dependency rule with more prescribed rings.
- **No prescribed shape** — cheapest now; the first erosion goes unnoticed, and the shape is what
  entropy erodes first.

## Sources / Prior art

- Alistair Cockburn, *Hexagonal Architecture* —
  <https://alistair.cockburn.us/hexagonal-architecture/>.
- Robert C. Martin, *The Clean Architecture* —
  <https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html>: source code
  dependencies point inward only.
- The earlier code base on `main`: its ADRs on ports and adapters and on the separate
  `adapter-git` crate.

## Consequences

- Positive: the layout and the operations are tested without a repository; a front end or a Git
  access is added or replaced without touching `core`; Q-4 is held in the one adapter that reaches
  repositories.
- Negative / trade-offs: a port for every way `core` reaches out, and a crate per adapter; an
  operation that `git` does natively is phrased through a port first.
- Follow-ups: the Git access behind the repository port, which is ADR-0019; what counts as a
  technology in Rust, stated in the conventions with the golden path.

## Enforcement

A structural test in `crates/core/tests/structure.rs` reads the manifest of `ocelli-core` and
fails when it names a dependency outside its allowlist of pure libraries, a workspace crate
included; it cites this ADR with a `Verifies:` marker. Cargo itself refuses a dependency cycle,
so an adapter cannot be reached from `core` without a declared dependency. That a port is no
wider than its callers need, and that `core` speaks no adapter's vocabulary, is review.
