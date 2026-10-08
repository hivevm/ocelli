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

- **A test looks only for what it started.** A process or a file it checks for carries its own
  run's mark, its scratch directory or its process group: a machine-wide search sees another
  session's run.
- **Logic lives in a pure helper beside the component, not inside it.** Grouping refs into
  folders, choosing the rows to ask the host for, rounding an edge's path through its corners,
  deriving a file tree, and pairing diff lines are each a function over plain data in a module of
  its own (`refTree.ts`, `rowWindow.ts`, `edgePath.ts`, `fileTree.ts`, `diffSplit.ts`), and the
  component only draws what it returns; so is ordering a host's answers, which drops one from an
  older load or overtaken by a newer request (`hostAnswers.ts`), and matching a commit's ref chips
  to the branches and tags they switch to or delete (`refChips.ts`). What the core decides, such
  as which transfer the toolbar offers and which operations a commit does, the host sends and the
  component draws
  ([ADR-0036](adr/0036-desktop-host-answers-what-the-core-decides.md)). A
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
- **A test that something did not happen first shows that it could have.** A test that no
  credential was sent asserts that the server asked for one, over a transport that asks, which
  the local file transport never does; a test of progress asserts that some was reported; a test
  of a refusal accepts only the reason of the guard it is about, not that of another guard in
  front of it. What a library does on its own, such as libgit2 sending the user name an address
  carries, is read off the library run on the same input, never recalled. A test that cannot fail
  is true by construction and guards nothing.
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
- **A write's tests read what `git` does off `git` run on the same input, never recalled.** A
  write does what `git` does for the same command, refusing first where `git` would or a listed
  departure decides ([ADR-0033](adr/0033-writes-what-git-writes.md)); libgit2 reads none of
  the index, the operation in progress, or the configuration keys `git` weighs for the caller,
  so each is a state its tests set up. A departure is tested against `git` doing otherwise. A
  refusal's test compares everything the write can change before and after: `HEAD`, the refs,
  the index, the working directory's status with ignored files, the configuration, the stash,
  and an operation in progress, the one snapshot `refused` in `crates/git/tests/support/mod.rs`
  takes. `clippy.toml` of the git adapter refuses the methods that take a path as a pattern or
  write the working directory past libgit2's checkout, and a call that stays says why. `git`'s
  state is more than the repository's state: a sequencer, an autostash, a file changed since an
  operation stopped. A write's tests include a path with `[` or `*` in it, a link, an
  executable file, and each such state.
- **A control that writes takes no second press while its write runs, and offers no stop it
  cannot keep.** A second click on a write that has not answered yet writes twice, or is refused
  for a state the first one changed; a Cancel on a push, which libgit2 cannot stop once its
  objects are sent, tells the developer that nothing was pushed when it was.
- **What a repository's configuration or layout chooses is untrusted wherever it is passed on.**
  A check that a repository stays inside a boundary names each thing libgit2 reads or follows on
  its own, as the pinned libgit2 parses it (a `.git` file, `commondir`, `core.worktree`,
  alternates and how a line ends, a link anywhere in the git directory, a submodule's git
  directory, a search upwards), and its tests build each one as a way out. A remote's
  address and the user name in it are checked for what the receiving side reads as structure, a
  line break for a credential helper, before they reach it, as CVE-2020-5260 showed; where two
  parsers read one value, as git2 and libgit2 read an address, it goes on only where both read
  it alike. A cloned repository's configuration is written by anyone, and the developer's
  credentials are the asset it is after.
- **Configuration that grants a capability carries a test that reads it.** The WebView's content
  security policy and the lockfile set are asserted against the files that ship
  (`src-web/src/hardening.test.ts`), because a regression in a JSON file is silent.
