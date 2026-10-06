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
  and the desktop host takes the path on its command line as `ocelli-cli` takes its own, so that
  a change to what a behaviour is touches both and the two cannot drift into two products,
  for the reason the shared core exists ([ADR-0009](adr/0009-ports-and-adapters.md)).
- **A test is named as the property it asserts, in a sentence.**
  `parents_are_always_below_their_children`, not `test_sort`: the name is what a reader sees when
  it fails, and the suite is the contract the decisions are held to
  ([ADR-0002](adr/0002-decisions-verified-by-tests.md)).
- **A comment says why, and cites the ADR where the shape follows a decision.** The line says what
  it does; the citation is how the next session finds the reasoning instead of taking a guarded
  loop for an accident, which matters most in `crates/core/src/graph.rs`. A state an accepted ADR
  rules out gets the citation where a reader would expect a guard, not a guard: a check that can
  never fire is code no test can reach.
- **A test's input is one its producer can make, and each outcome gets a case.** Code that reads
  the core's layout or git's output is tested on layouts `build_graph` lays out, on graphs git
  drew, or on hand-made ones only in shapes the governing ADR allows, the synthetic nodes of
  [ADR-0024](adr/0024-synthetic-nodes-in-the-first-chunk.md) included where they reach the code;
  a fixture nothing produces tests a case that cannot occur. A script that reports a verdict is
  tested on each verdict and at its boundary, since a flipped or truncated comparison passes a
  test that sees one side only.
- **A failure is reported, never dropped or read as a value.** In the web interface a failed
  command sets the status bar; in a script a failure exits non-zero and says what failed. A count
  that cannot be read, or a measurement taken before the state it measures was reached, is a
  failure, not a zero or a pass.
- **Configuration that grants a capability carries a test that reads it.** The WebView's content
  security policy and the lockfile set are asserted against the files that ship
  (`src-web/src/hardening.test.ts`), because a regression in a JSON file is silent.
- **A rule changes everywhere it is stated** — a change to a rule, a step, or a name searches
  the tree for its old wording and for every citation of its anchor, and corrects them in the
  same change. A link check sees the anchor, never a sentence that restates it.
- **A self-test asserts what it builds** — it takes its fixtures from the tree it copies or writes
  them itself, never from the template's own ADRs, scripts, or numbers, so it holds in a derived
  project and after a normalization.
