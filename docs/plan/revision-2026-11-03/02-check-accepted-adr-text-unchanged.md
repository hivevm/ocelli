# Ticket: A check refuses a change to the text of an accepted ADR

<!-- Filed by the design revision of 2026-11-03. Decided in docs/adr/0004-feature-layer.md. -->

## What to build

Commit 076d780 rewrote two paragraphs of ADR-0031, accepted two days before, in a commit that also
landed the merge editor's code. [`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 5 forbids it
outside a normalization (two human edits aside, the `Status` line and the `Deciders` line), and rule
4 keeps a commit that changes an ADR's text free of anything else. A sensor lists the second;
nothing refuses the first, and
[`AGENTS.md` §5](../../../AGENTS.md#5-quality-bar--definition-of-done) says a mistake a check could
have caught becomes a check.

A check, in `scripts/check-docs.sh` or a script of its own run by `scripts/check-all.sh`, refuses a
branch whose diff against its merge base with main changes an ADR that is `accepted` at the merge
base anywhere but its `Status` and `Deciders` lines. A normalization, which rewrites and renumbers
the whole record, and the template repository, which corrects its own ADRs in place (ADR-0005),
pass: how the check tells a normalization apart (a marker in its commits, the renumbering itself, or
an instruction the human gives) is the first question for the human.

## Acceptance criteria

- [ ] Given a branch that changes a sentence of an accepted ADR, when the check runs, then it fails
      and names the ADR and the line.
- [ ] Given a branch that flips a `proposed` ADR's `Status` to `accepted`, or adds to the `Deciders`
      line of an inherited one, when the check runs, then it passes.
- [ ] Given a branch that changes the text of a `proposed` ADR, when the check runs, then it passes.
- [ ] Given a normalization, when the check runs, then it passes by the means the human chose.
- [ ] The check's self-test covers each verdict and its boundary (docs/CONVENTIONS.md, "A test's
      input is one its producer can make").

## Read first

- [`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rules 4 and 5;
  docs/adr/0002-decisions-verified-by-tests.md; docs/adr/0005-template-releases.md
- `scripts/check-docs.sh` and its self-test, `scripts/sensor-review-binding.sh` (which already reads
  the ADR-only commits), `scripts/check-all.sh`
- docs/CONVENTIONS.md, "A self-test asserts what it builds"

## Blocked by

- None (can start immediately)

## Mode

with the human — the check touches what every agent hits and the normalization's exemption is a
design choice; it may also belong upstream in the NUC template, whose checks these are.
