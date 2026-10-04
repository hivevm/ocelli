# Conventions — Ocelli

> How this project writes code and tests **where no check decides it**: the judgement calls. What
> a module may know, what a test asserts and never does, the shape of an error, how a thing is
> named when the glossary has no word for it. Whoever settles a convention updates this file
> inline, an agent included, in the same change as the review or code that settled it. The
> reviewer always has it ([ADR-0001](adr/0001-agent-governance-model.md)).

An entry says in a sentence or two what is done and why; the why tells a convention from a habit.
A convention a check can decide is the check, not an entry. One that constrains future choices is
an ADR. What the golden path already shows needs none. Few entries that hold beat many that are
skimmed. The design revision drops what prevented nothing
([`AGENTS.md` §5](../AGENTS.md#5-quality-bar--definition-of-done)).

- **Behaviour carried over is pinned before it moves.** Code that takes over behaviour from the
  reference implementation lands with a reference test that compares against that
  implementation's own output, captured before the move; a test written after the move only
  restates the new code.
- **Logic lives in a pure helper beside the component, not inside it.** Grouping refs into
  folders, computing a lineage, filtering a history, deriving a file tree, and pairing diff lines
  are each a function over plain data in a module of its own (`refTree.ts`, `ancestry.ts`,
  `search.ts`, `fileTree.ts`, `diffSplit.ts`), and the component only draws what it returns. A
  component needs a browser to test and a pane a terminal; a function needs neither.
- **A behaviour both front ends offer is ported, not reinvented.** The terminal application's
  search, lineage, file tree, and diff split follow the web interface's helpers module for module,
  so that a change to what a behaviour is touches both and the two cannot drift into two products,
  for the reason the shared core exists ([ADR-0009](adr/0009-ports-and-adapters.md)).
- **A test is named as the property it asserts, in a sentence.**
  `parents_are_always_below_their_children`, not `test_sort`: the name is what a reader sees when
  it fails, and the suite is the contract the decisions are held to
  ([ADR-0002](adr/0002-decisions-verified-by-tests.md)).
- **A comment says why, and cites the ADR where the shape follows a decision.** The line says what
  it does; the citation is how the next session finds the reasoning instead of taking a guarded
  loop for an accident, which matters most in `crates/core/src/graph.rs`.
- **Configuration that grants a capability carries a test that reads it.** The WebView's content
  security policy and the lockfile set are asserted against the files that ship
  (`src-web/src/hardening.test.ts`), because a regression in a JSON file is silent.
