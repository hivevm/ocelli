# ADR-0036: The desktop host answers what the core decides from the refs, and the web interface restates none of it

- **Status:** 🟢 accepted
- **Date:** 2026-10-08
- **Deciders:** Markus Brigl
- **Applies to:** the `load-refs` event and the `select_row` and `selection` answers of `crates/commands/src/` and `src-tauri/src/`, the transfers in `crates/core/src/transfer.rs`, the history operations in `crates/core/src/operations.rs`, the toolbar and commit detail of `src-web/src/`, and the fixtures in `crates/core/tests/fixtures/`

## Context

The core decides which fetch, pull, and push the refs make possible, or why there is none, and
which of them can be stopped while it runs (`RepoRefs::fetch`, `pull`, `push`, and
`Write::can_stop` in `crates/core/src/transfer.rs`,
[ADR-0035](0035-remotes-through-libgit2-transports.md)), and which history operations of G-5 a
commit offers on the checked-out branch (`RepoRefs::history_operations` in
`crates/core/src/operations.rs`, [ADR-0033](0033-writes-what-git-writes.md)). The terminal
application calls these directly. The web interface restates both in TypeScript,
`src-web/src/transfers.ts` and `src-web/src/operations.ts`, held to the core by the fixtures
`transfers.json` and `operations.json` that the core's `tests/write_fixture.rs` writes for a
handful of sets of refs.

So the logic lives twice, and the fixtures check only the cases the core's test picks: a case
the core adds and the fixture does not sample drifts silently, against the convention that a
behaviour both front ends offer lives once in the core. The desktop host already holds the refs
of each load, which it sends as the `load-refs` event, and already answers a core question for
the web interface by a command: `write_discards` says what a write discards
([ADR-0034](0034-discarding-work-only-after-confirmation.md)). Selecting a commit is a command
whose answer, `Selected`, the host computes from the history it keeps
([ADR-0031](0031-front-ends-read-row-windows.md)).

The fixtures of the wire format are another matter: `writes.json` and `files.json` hold the web
interface's types to the JSON the core takes and sends, which the web interface must spell
whatever answers it draws.

## Decision

We will have the desktop host send the core's answers computed from the refs: the transfers the
toolbar offers ride on the `load-refs` payload beside the refs, each a write with whether it can
be stopped or the reason there is none, and the history operations a commit offers ride on the
host's answer to selecting it, so that the web interface draws what it is sent and restates none
of the decisions; `transfers.ts`, `operations.ts`, their tests, and the fixtures `transfers.json`
and `operations.json` with the tests that write them go.

- **The transfers** are the core's `Transfers` of the refs: `fetch`, `pull`, and `push`, each
  `{ write, canStop }` or `{ reason }`, computed by the core once per load from the refs the same
  payload carries.
- **The history operations** are part of `Selected`, the answer of `select_row` and `selection`:
  `{ operations }` or `{ reason }` for a commit, none for a stash or the working directory, and
  none until the load's refs are read, after which the web interface asks for the selection
  again. The reset among them is mixed; the mode the developer chooses in the commit detail stays
  the web interface's to set.
- **The wire format** of what the host sends is held to the web interface's types by a fixture the
  core writes, as `writes.json` and `files.json` are, which stay.

**Out of scope:** what the terminal application calls, which already asks the core directly; other
answers the web interface derives from the refs, such as the ref chips of `refChips.ts`, which a
later decision can move the same way.

## Alternatives considered

- **Keep the mirrors held by fixtures, as now** — the web interface answers without a round trip,
  but the logic lives twice, each new case of the core is a fixture case someone must remember to
  add, and a case the fixture does not sample drifts unseen.
- **A command per answer, as `write_discards` is** — keeps the payloads as they are, but adds a
  round trip and a stale-answer guard for each, for answers the host can compute when it sends
  the refs and the selection anyway, from the same state.
- **Operations on `select_commit`'s answer** — `select_commit` loads the changed files and emits
  them as an event without an answer of its own; `Selected` is the answer the web interface
  already checks against its load, so the operations arrive with the commit they belong to.

## Sources / Prior art

- The convention that a behaviour both front ends offer lives once in the core,
  [`docs/CONVENTIONS.md`](../CONVENTIONS.md).
- Design revision no. 7, which found the two mirrors and drafted this decision.
- The `write_discards` command of `src-tauri/src/commands.rs`, the host answering a core question
  for the web interface.

## Consequences

- Positive: the web interface has no transfer or operation logic to keep in step; a case the core
  adds reaches both front ends at once, and nothing samples it.
- Positive: the toolbar and the commit detail draw an answer computed from the same refs the
  sidebar shows, sent in the same payload.
- Negative / trade-offs: the payloads grow by a few writes per load and per selection; a commit
  selected before its load's refs are read shows its operations only once the selection is asked
  again, which the web interface does on `load-refs`, and on an answer without them that comes
  after the refs.
- Negative / trade-offs: what the host sends is a wire format of its own, held by one more
  fixture.
- Follow-ups: whether the ref chips' switch and delete move to the host the same way.

## Enforcement

The core's tests of `crates/core/src/transfer.rs` and `crates/core/src/operations.rs` cite this ADR
for the shape the answers serialize to; the host's tests in `crates/commands/src/session.rs` and
`crates/commands/src/lib.rs` cite it for the operations on the selection and the transfers on the
`load-refs` payload; the core's `tests/write_fixture.rs` writes the wire format to
`crates/core/tests/fixtures/offers.json`, which the web interface's `src-web/src/writes.test.ts`
reads against its types. That the web interface restates no other part of these answers is held
by review.
