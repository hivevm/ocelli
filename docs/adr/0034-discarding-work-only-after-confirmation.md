# ADR-0034: An operation that discards work runs only after the developer confirms it, the core naming which do

- **Status:** 🟢 accepted
- **Date:** 2026-10-07
- **Deciders:** Markus Brigl
- **Applies to:** the writes of the `RepositoryWriter` port in `crates/core/src/port.rs`, and how every front end calls them

## Context

Q-6 asks that an operation that discards work cannot happen by accident: dropping a stash, a hard
reset, a force push, deleting a branch that is not merged runs only after the developer confirms
it. G-7 brings the first of them. Two front ends call the same writes through the core's writer
port ([ADR-0033](0033-writes-what-git-writes.md)); were each to decide for itself which writes
need a confirmation, the two would drift, and a third, the remote server, would have to learn the
list again.

Of the writes of G-7, dropping a stash, applying a stash and dropping it after, and deleting a tag
discard work: a deleted tag comes back only by its SHA, and an annotated one loses its message.

## Decision

We will have the core name, for every write, whether it **discards work**, and accept a write
that does only together with the developer's **confirmation**, which every front end asks for in
words that name what is lost before it calls the write.

- **The core decides which.** A write's kind says whether it discards work; dropping a stash,
  applying and dropping one, and deleting a tag do; the writes of G-4 and G-5 join the list as they
  come.
- **The core refuses without it.** A write that discards work and comes without a confirmation is
  refused before it reaches the repository, so a front end that forgets to ask cannot discard
  anything.
- **The front ends ask.** The desktop application with a dialog whose default button is the one
  that keeps the work, the terminal application with a prompt that takes only an explicit yes.

**Out of scope:** undoing a write after the fact; a setting that turns the confirmation off.

## Alternatives considered

- **Each front end decides** — no list in the core, but the front ends drift, and one that
  forgets discards work.
- **Undo instead of asking** — less friction, but a dropped stash or a deleted tag would have to
  be kept somewhere to come back, which libgit2 does not do for us, and a force push cannot be
  undone at all.
- **A confirmation for every write** — nothing slips through, but confirming what loses nothing
  trains the developer to confirm without reading.

## Sources / Prior art

- Q-6 of the specification, which names the operations.
- `git stash drop` and `git tag -d`, which ask nothing and lose what they remove from reach —
  <https://git-scm.com/docs/git-stash>, <https://git-scm.com/docs/git-tag>

## Consequences

- Positive: no front end can discard work without asking (Q-6); which writes discard work is
  said once; the dialog and the prompt name what is lost.
- Negative / trade-offs: one more step for the writes that discard work; the core's write
  requests carry a confirmation.
- Follow-ups: the writes of G-4 and G-5 named as they come.

## Enforcement

The tests of the core cite this ADR with a `Verifies:` marker: each write that discards work is
named so, and one sent without a confirmation is refused with the repository unchanged. The tests
of both front ends cite it for asking before such a write and for calling nothing when the
developer declines.
