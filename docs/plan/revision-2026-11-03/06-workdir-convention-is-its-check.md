# Ticket: The convention on `workdir` keeps only what its check does not decide

<!-- Filed by the design revision of 2026-11-03. Decided in docs/adr/0004-feature-layer.md. -->

## What to build

docs/CONVENTIONS.md gained, since the last revision (commit fc94518), the entry "Every write of the
working directory goes through `workdir`, behind its guard": 25 lines that restate the module
documentation of `crates/git/src/workdir.rs` nearly sentence for sentence, while
`crates/git/clippy.toml` refuses every such call outside `workdir`. The file's own preamble says "A
convention a check can decide is the check, not an entry", and the design revision drops what
prevented nothing ([`AGENTS.md` §5](../../../AGENTS.md#5-quality-bar--definition-of-done)). During
the merge editor's work the save went through `workdir::checkout_index` because clippy refuses the
alternative; the entry's prose was not what held it.

Shrink the entry to what neither clippy nor the module's documentation says, if anything, with a
pointer to `workdir.rs` and ADR-0032; or drop it. No ADR cites the entry.

## Acceptance criteria

- [ ] Given docs/CONVENTIONS.md, when a reader looks for why every working-directory write goes
      through `workdir`, then one place says it (`workdir.rs`), and the convention, if it stays,
      says only what that place and `clippy.toml` do not.
- [ ] `scripts/check-all.sh` passes.

## Read first

- docs/CONVENTIONS.md, the preamble and the entry; `crates/git/src/workdir.rs`, its module
  documentation; `crates/git/clippy.toml`
- docs/adr/0032-conflict-labels-path-safe.md

## Blocked by

- None (can start immediately)

## Mode

with the human — the entry was added by the maintainer weeks ago, and whether its prose still
carries something the code does not is his call.
