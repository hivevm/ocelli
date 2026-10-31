# ADR-0033: Ocelli writes through libgit2 what `git` writes for the same command, refusing first where `git` would or an ADR departs, and running no program

- **Status:** ⚪ superseded by ADR-draft-merge-editor
- **Date:** 2026-10-08
- **Deciders:** Markus Brigl
- **Applies to:** the `RepositoryWriter` port in `crates/core/src/port.rs`, the writes of `crates/git/src/` in `write.rs`, `guard.rs`, `checkout.rs`, `workdir.rs`, `message.rs`, `operations.rs`, `conflicts.rs`, and `stash.rs`, the working directory a pull's fast-forward writes from `remote.rs`, the reading of a stopped operation in `refs.rs`, every call of the writer from a front end and how a front end shows a stopped operation, `crates/git/tests/git_parity.rs`, and the tests of Q-4

## Context

G-12, G-13, G-14, and G-16 write to a repository: tags, stashes, the index, commits, branches, the
history operations merge, rebase, cherry-pick, revert, and reset, and the resolution of a
conflict one of them stopped on. The developer knows `git`, and the repository is one `git`
works on before and after Ocelli: a write that does otherwise than `git` surprises the developer
or leaves a state `git` cannot carry on with.

Q-4 asks that opening a repository runs no code the repository brings, and a write is one more
way to run it: `git` runs hooks, the programs its configuration names, a merge tool or an editor,
and a signing program where `commit.gpgSign`, `tag.gpgSign`, or `tag.forceSignAnnotated` asks
for one. Q-7 asks that no work is lost by accident: `git` itself drops a stopped operation with
`git stash`, and deletes an ignored file a switch puts something in the place of. An ignored
file is often the developer's own: a local configuration, a build's output, or a note kept out of
the history on purpose.

libgit2 writes otherwise than `git` in corners: it reads none of the configuration a command of
`git` weighs, a checkout can stop halfway, a cherry-pick can commit a tree without a file it
brings, its rebase keeps its steps in other files than `git`'s, and its stash applies only over
an index without a staged change. A write acts on what the developer saw, while another tool can
change the repository underneath, and after a write what the front ends show must follow; the
history loads in chunks into a load session ([ADR-0016](0016-history-loads-in-chunks.md),
[ADR-0031](0031-front-ends-read-row-windows.md)). After a commit, the developer looks for the
commit just made. The working-directory node they committed from may be gone, or still hold what
was left unstaged.

## Decision

We will write to a repository only through a `RepositoryWriter` port of the core, implemented
with libgit2 through git2 in `ocelli-git`, each write doing what `git` does for the same command
and refused before anything changes where `git` would refuse it or a departure below decides
otherwise, with no write running a program.

### The writer

- **The port.** Every write goes through `RepositoryWriter`; `GitWriter` implements it in
  `ocelli-git`. Writes run where reads run, off the thread that draws
  ([ADR-0019](0019-git2-repository-reader.md)).
- **What `git` does.** Where libgit2 differs from `git`, a name `git` refuses, a message `git`
  cleans up, a file in the way `git` will not overwrite, the writer follows `git`. It weighs what
  `git` weighs for the command: the working directory, the index, an operation in progress, and
  every configuration key `git`'s documentation names for the command, the repository's
  configuration over the global one. Every repository Ocelli writes stays one `git` carries on
  with.
- **What a write expects.** Each write names the state the developer saw and is refused where the
  repository no longer holds it: a tag's name free and its commit there, or the tag at the commit
  shown; a stash entry at its place by its commit; a file's content in the working directory or
  the index by blob id; the checked-out commit for a commit; a branch at its tip and merged or not
  as shown; a remote branch at its tip; a conflict by the blob ids of the index's stages. A write
  that would leave the working directory in conflict, or overwrite a staged change, is refused.
- **A refusal changes nothing.** It is decided before libgit2 is called, since libgit2 can stop
  halfway. One only libgit2 can find, such as a branch name clashing with another, goes first,
  and a refusal after it undoes it. A write that waits, on a remote or on a lock, checks what it
  expects again after the wait, under a lock on what it moves.
- **Paths and links.** A path goes to libgit2 as written, never as a pattern. The working
  directory is written only through libgit2's checkout, a link as a link and a file with its
  mode, and nothing is written below a link or a file, inside the repository or out of it.
- **No program runs.** The writer reaches the repository through libgit2 alone: no hook,
  `core.hooksPath` or not, no program the configuration names, no merge tool, no editor, no
  signing program, no `git` binary.
- **Signatures.** A write `git` would sign is refused where the configuration asks for a
  signature, read as `git` reads it: a commit, an operation's commit included, under
  `commit.gpgSign`, an annotated tag under `tag.gpgSign` or `tag.forceSignAnnotated`; the reason
  names the setting and says to use `git`. A lightweight tag is written under `tag.gpgSign`: it
  carries nothing to sign, and refusing it would leave no way to mark a commit from Ocelli.
- **Who it writes as.** A write that records a person, a commit, an annotated tag, or a stash,
  takes the identity of the repository's configuration, and is refused where none is set.
- **After a write.** The writer returns the commit a write made; the front end opens a new load
  session and selects that commit, or else the commit selected before, where it still exists.

### The history operations

- **Where they start.** Merge, rebase, cherry-pick, revert, and reset start on the checked-out
  branch at its tip as shown, with no operation in progress, a bisect included, and no staged or
  unstaged change to a tracked file; untracked files may stay where the operation does not write.
- **Merge** merges the selected commit or branch into the checked-out one: a fast-forward where
  possible unless `merge.ff` is `false`, else a merge commit with `git`'s message, `Merge branch
  '<name>'`, `Merge remote-tracking branch '<name>'`, or `Merge commit '<sha>'`, followed by
  ` into <branch>` unless `merge.suppressDest` names the branch, `main` and `master` where it is
  unset. `merge.ff = only` refuses what cannot fast-forward.
- **Rebase** replays the checked-out branch onto the selected commit, without interaction,
  through libgit2's rebase on disk. `git status` reads a rebase of either side as one in progress
  and either side aborts the other's, but a rebase is continued only by the side that started it.
  A step that cannot be applied for another reason than a conflict, a file in its way, undoes a
  rebase just started whole; met while continuing, it is marked so that no later continue takes
  it for a step resolved to nothing, and the rebase can only be aborted.
- **Cherry-pick** applies the selected commit onto the checked-out one and commits it with its
  author and message; **revert** applies its inverse and commits it with `git`'s message, `Revert
  "<subject>"` and `This reverts commit <sha>.`. A merge commit is refused for both: which parent
  is the mainline is the developer's to say, with `git`. One that changes nothing on the branch is
  refused, with `git`'s state cleared.
- **Reset** moves the checked-out branch to the selected commit, soft, mixed, or hard; a hard
  reset discards work and runs only after the developer confirms it
  ([ADR-0034](0034-discarding-work-only-after-confirmation.md)).
- **On a conflict** the operation stops, leaving the conflicted index, the conflict markers in the
  working directory, and `git`'s state files, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, or
  the rebase's directory, as `git` would; nothing is committed. A merge, a cherry-pick, or a
  revert stopped so is carried on by Ocelli or `git` alike.

### A stopped operation

- **What is shown.** The kind of operation in progress, read from `git`'s state whoever started
  it, and its conflicted files; a front end shows them wherever a repository is open.
- **Resolving a file.** Ours or theirs writes that side to the working directory and the index;
  it replaces what the developer may have edited in that file, so it runs only after they confirm
  it. As edited takes the working directory's file, refused while it still holds a conflict
  marker line (`<<<<<<<`, `=======`, `>>>>>>>`). A path below a link or a file in the working
  directory is refused.
- **Continuing** is refused while a file is conflicted. A merge commits with both parents and the
  message `git` prepared, cleaned up as `git merge --continue` cleans it up; a cherry-pick or a
  revert commits as it would have without the conflict; a rebase Ocelli started commits the step
  and goes on, stopping again at the next conflict, a step resolved to no change dropped as
  `git rebase --continue` drops it; one `git` started, whose steps libgit2 cannot read, is refused
  with the advice to continue it with `git`. The state files go once the operation is done.
- **Left to `git`:** a sequence of cherry-picks or reverts `git` started, whose later commits only
  its sequencer knows, and an operation holding an autostash, which `git` puts back when it ends;
  continuing or aborting either is refused with the advice to do it with `git`.
- **Aborting** puts the branch, the index, and the working directory back as they were before the
  operation, as `git <operation> --abort` does, a rebase from the `head-name` and `orig-head` that
  `git` and libgit2 both write; it discards the resolutions made, so it runs only after the
  developer confirms it ([ADR-0034](0034-discarding-work-only-after-confirmation.md)). A file the
  operation brought in or removed cleanly, which the developer changed or made since, makes the
  abort refuse, as `git merge --abort` refuses, rather than lose it, a file made inside a
  directory in its place, or in a conflicted file's place, included. Below a link or a file
  nothing is written, one the operation itself put there moved aside first. An abort libgit2
  cannot finish keeps the operation stopped with its conflicts.

### Stashes

- **Making a stash** is refused where an operation is in progress, a bisect aside, and where
  libgit2 would write past what it stashes: a link where the index has a file or a directory,
  through which it writes the file's committed content wherever the link points; an entry the
  index marks skip-worktree, assume-unchanged, or intent-to-add, whose content it overwrites or
  misreads; and, with the untracked files, an ignored file in a directory only untracked files
  make, which it deletes without stashing it.
- **Applying or popping** a stash is refused where an operation is in progress, where the index
  holds a staged change, the refusal saying to commit or unstage first, and where the working
  directory deleted a file the stash changes, the refusal naming the file.
- **Everything else** a stash does follows `git`: the working directory and the index end as
  `git stash` leaves them.

### Files the index does not track

- **No write deletes one.** A write that would put a file or a directory where the working
  directory holds a file the index does not track, ignored files included, is refused before
  libgit2 writes anything; the refusal names the file and says to move it first.

### Where Ocelli departs from `git`

Each departure is a refusal with nothing changed, or a write `git` would make otherwise, and
`crates/git/tests/git_parity.rs` lists the ones it compares in `DEPARTURES`, citing this ADR.

- **Hooks, programs, and signatures.** No hook, configured program, merge tool, editor, or
  signing program runs. A commit, an annotated tag, a merge, a rebase, a cherry-pick, and a
  revert are refused where signing is asked, where `git` signs; under `tag.gpgSign` a tag is
  lightweight, where `git tag <name>` makes a signed annotated one.
- **A clean tree.** Merge, rebase, cherry-pick, and revert are refused over a change to a tracked
  file, which `git` carries along where it does not collide: a stopped operation never mixes the
  developer's changes with its own.
- **Nothing in progress.** The history operations, the resets included, are refused while an
  operation is in progress, a bisect included.
- **Concluding a stopped operation.** A commit is refused while a merge, a cherry-pick, a revert,
  or a rebase is stopped; continuing it is what commits it.
- **A stash in a stopped operation.** Making, applying, or popping a stash is refused while an
  operation is stopped, where `git stash` drops the operation with no question asked.
- **A stash over a staged change or a deleted file.** Applying or popping is refused over a staged
  change on any path and over a file the working directory deleted; over a staged change on a
  path the stash touches it is refused, where `git stash apply` stops in the conflict.
- **A stash libgit2 would make past what it stashes**, through a link, over a skipped,
  assume-unchanged, or intent-to-add entry, or deleting an ignored file, is refused.
- **An ignored file in the way** of a switch, a pull's fast-forward, a merge, a rebase, a
  cherry-pick, or a revert is refused, where `git` deletes it and goes on.
- **A refusal changes nothing** where `git` writes first and stops: `git switch -c` writing the
  target's tree before it finds the name taken, `git stash apply` writing the stash's untracked
  files or changes before it finds a file in its way.
- **Taking a side over a directory.** Ours or theirs is refused where the developer made a
  directory in the file's place, which `git checkout --ours/--theirs` deletes.
- **A merge's message.** Under `commit.cleanup` `whitespace` or `verbatim`, continuing a merge
  does not write the editor's template `git` adds into its message.
- **A rebase's steps.** A rebase is continued only by the side that started it, Ocelli or `git`.

A difference from `git` that no clause here decides is a mistake: the comparison lists it in
`MISMATCHES`, asserted to still differ, so that a fix removes it.

**Out of scope:** which writes discard work and how they are confirmed
([ADR-0034](0034-discarding-work-only-after-confirmation.md)); fetching, pulling, and pushing
([ADR-0035](0035-remotes-through-libgit2-transports.md)), but for the working directory a
pull's fast-forward writes; signing in Ocelli; interactive rebase; choosing a mainline; merge
strategies and options beyond libgit2's default; rebasing onto other than the selected commit;
`--autostash`; a merge editor inside Ocelli; resolving hunk by hunk; `git rerere`; skipping a
rebase step; applying a stash by hand where libgit2 refuses; `git stash push` of chosen paths;
`git stash branch`.

## Alternatives considered

- **The `git` command line** — behaves exactly as `git` does, but starts a program and runs
  every program the repository names, hooks, merge tools, and signing included, which Q-4 rules
  out.
- **libgit2 as it is, no comparison with `git`** — the least code, but a checkout that stops
  halfway, a cherry-pick without a file, and a configuration nobody reads, each a surprise for a
  developer who knows `git`.
- **Writing to what is there now** — no check to maintain, but the developer's choice can land on
  something else than they saw.
- **Write unsigned where signing is asked, with or without a warning** — no refusal, but an
  object `git` would not have written, found out when a protected branch refuses the push; a
  warning comes after the object exists, and a warning read after the fact is a warning missed.
- **Sign in Ocelli** — the signature without `git`, but Ocelli would hold or reach the
  developer's keys, an asset the specification keeps out of its hands.
- **Select the commit selected before, always** — after a commit it selects the
  working-directory node or nothing, and the developer looks for the commit by hand.
- **Updating the rows a write touched instead of loading again** — faster on a large history, but
  a second way to reach a laid-out graph beside the chunked load.
- **Carrying uncommitted changes along as `git` does** — less friction, but a conflict then mixes
  the developer's own changes with the operation's, and aborting it cannot tell them apart.
- **Running the operations in memory and refusing every conflict** — never leaves a stopped
  state, but nothing that conflicts could be merged or rebased from Ocelli, which G-14 asks for.
- **A state of Ocelli's own for a stopped operation** — free of `git`'s file formats, but `git`
  could not carry on with it, nor Ocelli with one `git` started.
- **An editor of conflicts in Ocelli, or the configured merge tool** — hunk by hunk where the
  developer reads them, but a component of its own in both front ends, for what a text editor and
  one of the two sides already do, or a program the configuration names (Q-4).
- **Resolving only what Ocelli started** — simpler, but a conflict `git` left would block the
  developer in Ocelli.
- **Following `git` in every state, stashes and ignored files included** — the fewest surprises,
  but a stopped operation dropped by a stash loses its resolutions, a stash applied beside a
  staged change needs one applied by hand past libgit2's checks, and an ignored file no commit
  holds is gone for good; a confirmation would ask about files the developer may not know are
  there.
- **Refusing only the stash during a stopped operation** — keeps the safety that matters most,
  but leaves applying beside a staged change or over a deleted file undecided, which the
  comparison with `git` then reports as mistakes.
- **A decision for each kind of write** — smaller records, but one sentence binds every writer
  module, and each record would restate it and name only part of the code it binds.

## Sources / Prior art

- libgit2's references, tags, stash, index, checkout, commit, merge, rebase, cherry-pick, revert,
  reset, and index conflicts (`git_index_conflict_get`) — <https://libgit2.org/libgit2/>,
  <https://docs.rs/git2>; `git_stash_apply`, which refuses an index with staged changes —
  <https://libgit2.org/docs/reference/main/stash/git_stash_apply.html>; `git_merge`, which writes
  `MERGE_HEAD` and removes it again where its checkout fails; `git_commit_create_with_signature`
  for a caller that signs, since libgit2 creates commits and tags without hooks or signatures
- `git commit`, `git switch`, `git checkout`, `git branch`, `git tag`, `git stash`, `git merge`
  and "How conflicts are presented", `git rebase`, `git cherry-pick`, `git revert`, `git reset`,
  `git add`, `githooks`, and `git-config` — <https://git-scm.com/docs>
- The comparison of every local write with `git`, `crates/git/tests/git_parity.rs`.

## Consequences

- Positive: one decision binds every writer module; a developer who knows `git` finds its
  behaviour, and every departure is listed in one place; writes run no code of the repository
  (Q-4); no write lands on a state the developer did not see, stops halfway, deletes a file no
  commit holds, or drops an operation in progress (Q-7); a stopped operation is one `git` and
  Ocelli both carry on with; after a commit the commit just made is in view.
- Negative / trade-offs: hooks a team relies on do not run; a repository that signs every commit
  cannot commit from Ocelli; uncommitted changes are committed or stashed before an operation; a
  merge commit is picked or reverted with `git`; libgit2's merge is not `git`'s `ort` in every
  corner; a rebase is continued by the side that started it; an edited resolution happens in the
  developer's own editor; a developer who stashes during a merge, applies a stash over a staged
  change, or has an ignored file in a target's way clears it first or uses `git`; a write on a
  large history waits for its first chunk again.
- Follow-ups: signing from Ocelli; the hooks of a commit, should a team need them; choosing a
  mainline; interactive rebase; skipping a rebase step; resolving hunk by hunk; applying a stash
  beside a staged change, should libgit2 allow it; the stash commits' messages, one line break
  longer than `git`'s, which the comparison lists as a mismatch.

## Enforcement

The comparison of every local write with `git`, `crates/git/tests/git_parity.rs`, cites this ADR
with a `Verifies:` marker: in every state that changes a write's outcome, the writer leaves what
`git` leaves, or refuses with nothing changed where a group of `DEPARTURES` citing this ADR says
so, and fails where a group decides nothing. The tests of the git adapter cite it, on
repositories made with `git`: each write compared with `git` on the same input; a write whose
expected state the repository no longer holds refused with nothing changed; a commit and an
annotated tag refused under `commit.gpgSign`, `tag.gpgSign`, or `tag.forceSignAnnotated`, in the
repository's configuration and in the global one, and a lightweight tag not; a repository whose
hooks and configured programs are set running none of them while written to; each history
operation compared with `git`, and a conflict leaving the state `git status` reads as such; the
conflicted files read as `git status` lists them, each way of resolving compared with
`git checkout --ours/--theirs` and `git add`, a path that reads as a pattern or lies below a
link, continuing and aborting compared with `git <operation> --continue` and `--abort`; the
stash's refusals. The core's tests name a hard reset, an abort, and taking a side as discarding
work; the tests of both front ends cite it for selecting the commit a write made after the
reload; the structural tests keep the writer off the thread that draws.
