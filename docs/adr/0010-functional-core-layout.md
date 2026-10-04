# ADR-0010: The layout of the commit graph is a functional core of pure functions over values

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** the layout module of the `core` crate and the tests that verify it

## Context

The layout is what Ocelli exists for: each branch in one lane (G-2), the main branch straight on
the left (G-3), fewer crossings than `git log --graph --all` (Q-1), and fast enough at one million
commits (Q-2). Each of these is a property of a function from commits to lanes and edges, and is
best tested exhaustively and on generated histories far larger than any fixture repository.

Ports and adapters (ADR-0009) keeps technologies out of `core`, but says nothing against I/O,
threads, or a clock inside it. From the catalogue: a functional core fits where the logic is rich
and the effects few, and where a quality goal asks for property-based tests; it does not fit
logic that is mostly effects in a sequence, which the layout is not.

## Decision

We will write the layout of the commit graph as **pure functions over values**: given the commits
and their parents, it returns the lanes, the rows, and the edges, and it performs no I/O, reads
no clock, starts no thread, and draws no randomness.

**Out of scope:** the layout algorithm itself, which is ADR-0015; the rest of `core`, which may
hold state; and how a front end draws the result.

## Alternatives considered

- **Ports and adapters alone** — keeps technologies out, but lets the layout read a repository
  incrementally or spawn threads, which makes it testable only through fakes.
- **No split inside `core`** — simplest while the logic is small; the layout is the logic that is
  not small.

## Sources / Prior art

- Gary Bernhardt, *Functional Core, Imperative Shell* —
  <https://www.destroyallsoftware.com/screencasts/catalog/functional-core-imperative-shell>.
- Gary Bernhardt, *Boundaries* — <https://www.destroyallsoftware.com/talks/boundaries>: values at
  the boundary make the core testable without mocks.

## Consequences

- Positive: G-2, G-3, Q-1, and Q-2 are decided by tests on generated histories of any size,
  without a repository; the same input always yields the same picture in every front end.
- Negative / trade-offs: incremental loading (Q-2) is the shell's job, feeding the layout in
  chunks it can process as values; parallelism, if the layout needs it, is a separate decision.
- Follow-ups: the layout algorithm, which is ADR-0015; the chunked feeding of a million commits,
  which is ADR-0016.

## Enforcement

A structural test in `crates/core/tests/structure.rs` scans the shipped source of the layout
module, `crates/core/src/graph.rs`, and fails on a use of `std::fs`, `std::io`, `std::net`,
`std::process`, `std::thread`, `std::time`, `std::env`, a clock's `now`, or a randomness crate;
it cites this ADR with a `Verifies:` marker. That the shell around the
layout stays thin is review.
