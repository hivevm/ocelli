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
  folders, choosing the rows to ask the host for, rounding an edge's path through its corners,
  deriving a file tree, and pairing diff lines are each a function over plain data in a module of
  its own (`refTree.ts`, `rowWindow.ts`, `edgePath.ts`, `fileTree.ts`, `diffSplit.ts`), and the
  component only draws what it returns; so is ordering a host's answers, which drops one from an
  older load or overtaken by a newer request (`hostAnswers.ts`). A
  component needs a browser to test and a pane a terminal; a function needs neither.
- **A behaviour both front ends offer lives once in the core; only drawing is ported.** Search,
  lineage, and a load's bookkeeping, its chunks, status, and growing search and selection, belong
  in the core for both front ends; the terminal application's file tree and diff split, which only
  draw, follow the web interface's helpers module for module,
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
  a fixture nothing produces tests a case that cannot occur. A generator yields every shape its
  producer can make, the rare ones its ADR names included, such as a date running backwards or a
  parent in a later chunk. A script that reports a verdict is
  tested on each verdict and at its boundary, since a flipped or truncated comparison passes a
  test that sees one side only. A test of a computed size or count uses an input past every floor
  or clamp applied to it, so a wrong count changes the outcome.
- **A property test excepts a case only where the specification or an ADR names the exception.**
  The exception cites that clause where it is made and counts that the excepted case occurs; an
  exception no clause names weakens the suite.
- **A failure is reported, never dropped or read as a value.** In the web interface a failed
  command sets the status bar, and ESLint rejects a `.catch` whose handler takes no error; in a
  script a failure exits non-zero and says what failed. A count that cannot be read, or a
  measurement taken before the state it measures was reached, is a failure, not a zero or a pass.
- **An answer is taken only while what asked for it still stands.** A result that comes back
  after an await is checked against the load, the commit, and the request it answers before it
  changes any state, and dropped where the developer or a reload has moved on; what is remembered
  across a reload, such as the commit to select again, is given up once the developer chooses
  another or the reload does not start. A write's result belongs to the repository, not to a
  load, so it comes back on a channel no reload replaces. A front end has two things moving at
  once, the developer and the repository, and an answer applied to the state that replaced its
  own shows one commit's result on another.
- **A write does what `git` does for the same command, and a refusal changes nothing.** Where
  libgit2 differs from `git`, a name `git` refuses, a message `git` cleans up, a file in the way
  `git` will not overwrite, the writer follows `git`, and its tests run `git` on the same input.
  A refusal is decided before libgit2 is called, since libgit2 can stop halfway, and its test
  compares the working directory's status, ignored files included, before and after. Every
  repository Ocelli touches stays one `git` knows what to do with.
- **A control that writes takes no second press while its write runs.** A second click on a write
  that has not answered yet writes twice, or is refused for a state the first one changed.
- **Configuration that grants a capability carries a test that reads it.** The WebView's content
  security policy and the lockfile set are asserted against the files that ship
  (`src-web/src/hardening.test.ts`), because a regression in a JSON file is silent.
