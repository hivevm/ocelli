# ADR-0040: A conflict's sides are labelled as `git` labels them, but for what would let a label spell a path

- **Status:** 🟡 proposed
- **Date:** 2026-10-10
- **Deciders:** Markus Brigl
- **Applies to:** the labels every write that can stop on a conflict gives libgit2 in `crates/git/src/workdir.rs`, the label calls `crates/git/clippy.toml` disallows elsewhere, and the label tests of `crates/git/tests/history.rs` and `crates/git/tests/write.rs`

## Context

[ADR-0033](0033-writes-what-git-writes.md) has Ocelli write what `git` writes for the same
command, refusing first where `git` would or an ADR departs. A merge, a rebase, a cherry-pick, and
a revert can stop on a conflict, and what they then write includes the labels of the conflict's
two sides.

`git` labels the two sides of a conflict in the markers it writes into a conflicted file, and,
where it cannot write a side at the conflicted path, a conflict between a file and a directory or
between two renames, in the name of the file it writes that side to, `<path>~<label>`. libgit2
does the same, and left to itself takes the label of the side applied from the commit's subject
for a cherry-pick, `parent of` it for a revert, and the step's subject for a rebase. A subject can
hold `/` and `..`, and libgit2 creates every directory the name `<path>~<label>` spells: where a
cherry-pick or a rebase stops on a collision of two renames, a commit whose subject climbs out of
the repository makes libgit2 write a new file anywhere the user can write.

Writing the labels `git` writes does not avoid this. `git` appends ` (<subject>)` to a commit's
label, and keeps a `/` in a branch's name in a marker, though it flattens that `/` to `_` in the
name of the file it writes a side to. libgit2 takes one label for both the marker and the name.

## Decision

We will label a conflict's sides as `git` labels them, but for what would let a label spell a
path, and refuse a write whose label is not path-safe before anything changes.

- **The labels.** The labels are given to every call that can write a conflict, so that libgit2
  never takes a commit's subject. The checked-out side is `HEAD`, for every operation. The side
  applied is the abbreviated commit for a cherry-pick and for each step of a rebase, its own, and
  `parent of` the abbreviated commit for a revert. For a merge it is the branch the developer
  named, where a local or a remote branch of that name points at the commit merged, its `/`
  flattened to `_` as `git` flattens it in the name of the file it writes a side to. The
  abbreviated commit labels a merge's side where no such branch is named, or where the name
  flattened is not path-safe.
- **The base.** `git`'s `diff3` and `zdiff3` styles write the base, labelled as `git` labels it:
  `parent of` the abbreviated commit for a cherry-pick and a rebase's step, the commit for a
  revert, and for a merge the merge base's abbreviated commit, `merged common ancestors` where
  there are several, and `empty tree` where there is none.
- **A stash** keeps the labels libgit2 and `git` both write, `Updated upstream` and
  `Stashed changes`.
- **Path-safe.** A label is path-safe where it holds no `/`, `\`, `..`, control character, NUL, or
  character Windows refuses in a name (`<`, `>`, `:`, `"`, `|`, `?`, `*`), does not end in a dot
  or a space, and is no longer than a name a file system takes. A commit's label always is, and a
  write whose label is not refuses before anything changes. The name of each file a side could
  be written to, `<path>~<label>`, is checked as any other path the write writes.

### Where the labels depart from `git`

- **The labels of a conflict's markers.** The label of a commit's side, and of a base that is a
  commit, is the abbreviated commit alone, where `git` appends ` (<subject>)`. A merge's branch
  keeps its `/` flattened to `_` in the marker too, where `git` flattens it only in the name of
  the file it writes a side to. Either would let a label spell a path. The comparison of a
  stopped operation's conflicted file with `git`'s, in `crates/git/tests/history.rs`, takes out
  exactly these differences and the two below.
- **The length of an abbreviated commit.** A label abbreviates a commit to seven characters, or to
  `core.abbrev` characters where it is a number, longer only where the prefix is not unique, as
  libgit2 abbreviates. `git` lengthens the default with the number of objects in the repository,
  so on a large one its labels are longer.
- **The base line of a file both sides added.** In the `diff3` and `zdiff3` styles libgit2 writes
  the base line bare where the file has no base. `git` labels it, `empty tree` in a merge of two
  histories with no base among them. The label given is not written.
- **A lock file in the way.** A write that would leave a conflicted file is refused where anything
  stands at that file's `<path>.lock`, where `git` writes the conflict all the same. libgit2
  creates the lock file exclusively to write a merged side and stops half done where one is there.

**Out of scope:** everything else a stopped operation writes and how it is resolved, which
ADR-0033 decides.

## Alternatives considered

- **The labels libgit2 takes by itself** — no code, but a commit's subject spells the path a side
  is written to, out of the repository where it climbs out.
- **Fixed words for every label, `ours` and `theirs`** — path-safe and simple, but the markers
  depart from `git`'s on every conflict, where a developer who knows `git` reads `HEAD` and the
  commit.
- **`git`'s labels with the subject cleaned of what spells a path** — the closest to `git`, but a
  subject cleaned is no longer the subject, and what a cleaning keeps is one more rule to get
  right. The abbreviated commit names the side as well and holds hexadecimal digits only.
- **A label per rebase, not per step** — one set of options, but every step's markers would name
  the same commit, where `git` names each step's own.

## Sources / Prior art

- libgit2 1.9.7's `cherrypick.c`, `revert.c`, `rebase.c`, `merge.c`, and `stash.c`, each of which
  takes a label only where the caller gives none, and `checkout.c`'s `checkout_path_suffixed`,
  which appends the label to the path, and `blob_content_to_file`, which writes a merged side
  through `<path>.lock`.
- `git` 2.55.0, observed: `<<<<<<< HEAD` on every operation; `>>>>>>> <abbreviated> (<subject>)`
  for a cherry-pick and a rebase, `>>>>>>> parent of <abbreviated> (<subject>)` for a revert,
  `>>>>>>> feature/x` for a merge of `feature/x`; and a side of a conflict between a file and a
  directory written to `x~feature_x`, the `/` flattened. In the `diff3` and `zdiff3` styles the
  base `||||||| parent of <abbreviated> (<subject>)` for a cherry-pick and a rebase,
  `||||||| <abbreviated> (<subject>)` for a revert, `||||||| <abbreviated>` for a merge with one
  base, `||||||| merged common ancestors` with two, and `||||||| empty tree` with none. A conflict
  written on `x` where an untracked `x.lock` stands, the lock file left as it was.
- libgit2 1.9.7's `merge_file.c`, which labels the base line only where the file has a base, and
  `object.c`'s `git_object__short_id` with `config_cache.c`'s `core.abbrev`, seven characters
  where it is unset or `auto`, its number where it is one.

## Consequences

- Positive: no commit's subject or branch's name decides where a side of a conflict is written,
  and the markers read as `git`'s but for the subject and a branch's `/`.
- Negative / trade-offs: a developer who looks for the subject in a marker finds the commit by its
  abbreviated id instead; a rebase is opened again before each step, to give it its own label,
  which libgit2 reads from the options it was opened with.
- Follow-ups: none.

## Enforcement

`crates/git/tests/history.rs` cites this ADR with a `Verifies:` marker. It compares the conflicted
file a merge, a cherry-pick, a revert, and a rebase leave, the rebase as it starts and on a
continue, with what `git` leaves on a twin, in the default, `diff3`, and `zdiff3` styles, a
criss-cross merge, a merge with no base, and a numeric `core.abbrev` among them, taking out only
the departures above. It checks that `git` names a side's file as a merge's side is labelled, and
that a name not a branch at the commit, or not path-safe, labels by the commit.
`crates/git/tests/write.rs` cites it for a commit whose subject climbs out of the repository
leaving nothing outside it, a rebase's markers holding `HEAD` and each step's commit, and the
guard refusing a link beside a conflicted path, found as the file system folds names, and
anything at its `.lock`. `crates/git/clippy.toml` allows the calls that set a label only inside
`workdir.rs`, where every label passes the check that refuses one spelling a path.
