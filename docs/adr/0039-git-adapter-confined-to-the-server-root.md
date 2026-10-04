# ADR-0039: The git adapter, confined to the server root, opens nothing whose files lie outside it

- **Status:** 🟢 accepted
- **Date:** 2026-10-08
- **Deciders:** Markus Brigl
- **Applies to:** every path a browser names to `ocelli-server`, the confinement of the git adapter in `crates/git/src/confine.rs` and the reads it changes in `refs.rs` and `git.rs`, the server root and its folder listing in `crates/server/src/root.rs`, the check `Host::guarded` in `crates/commands` makes before every command, what `crates/server/src/socket.rs` lets leave the server, and the sidebar's submodule, worktree, and parent rows it serves

## Context

The desktop host opens any path the developer picks in a native folder dialog or names on the
command line, and the git adapter finds the repository by searching upwards from it. The sidebar
switches to a submodule, a worktree, or the superproject a repository sits in
([ADR-0027](0027-submodules-and-repository-switching.md)). In the desktop that is the developer's
own hand on their own machine.

The remote server takes paths from a browser
([ADR-0037](0037-remote-server-websocket-per-tab.md)), behind its authentication
([ADR-0038](0038-remote-server-localhost-tls-session-cookie.md)). Q-5 guards who gets in;
what an authenticated browser can then reach is a second line. The machine the server runs on is
an asset (Threats), and the server shows file content: changed files, diffs, and the working
directory's content. A stolen session, or a mistake in the web interface, should reach only what
the developer meant to show.

A repository can point outside itself in more ways than its paths: a `.git` file or link, a
worktree's common directory, `core.worktree`, a link anywhere inside its git directory, which
libgit2 reads through wherever it leads, and the object stores `objects/info/alternates` borrows,
which libgit2 reads by rules of its own. A search upwards leaves the folder it started in, and
every read of the adapter opens the repository again, so a check made once at the server's door
holds only where every open of the adapter keeps to it. What git says when a read fails names
paths on the server. And the files on disk change after a repository was checked.

## Decision

We will confine the git adapter, for the whole process of `ocelli-server`, to the server root
named at its start, and open for a browser only a repository whose working directory, git
directory, common directory, and object stores all resolve below that root, links resolved,
with no link inside its git directory, checked again before every command that reaches it.

- **The server root.** Named at each start and required: there is no default. It is resolved
  once, links included, to the canonical path every check compares with. The server root itself
  may be a repository.
- **The confinement is process-wide.** The server sets it once at its start, before it serves
  anything, and nothing undoes it; the desktop application and the terminal application never set
  it. Confined, every open of the adapter, `GitReader`'s and `GitWriter`'s alike, takes the
  folder it is given as the repository's own with `NO_SEARCH`, so a folder whose `.git` vanished
  after the check fails instead of opening the repository around it.
- **Paths from the browser** are relative to the server root. The server joins one with the
  server root, resolves it, links included, and refuses it unless the result lies at or below the
  server root; `..`, an absolute path, and a link out of the server root are refused alike, with
  one reason that names what the browser sent and no path outside.
- **Opening a repository.** The folder named must be the repository's own working directory: a
  folder inside a repository, or inside none, opens nothing, and a bare repository opens nothing.
  The git directory, the common directory, and the working directory, `core.worktree` included,
  must resolve below the server root.
- **Alternates** are read as the pinned libgit2 reads them: lines split at `\r` and `\n`, empty
  lines and comments skipped, a line starting with `.` relative to the store that names it, an
  absolute line as it stands. A relative line without the leading `.`, which libgit2 reads
  against the process's working directory, a line with a control character, and a file that is
  not UTF-8 or cannot be read refuse the repository. Alternates of alternates are followed to
  libgit2's own depth of five, and a store named deeper refuses it. Every store must resolve
  below the server root.
- **No link in the git directory.** The git directory and the common directory are walked once
  when the repository opens, and a link anywhere inside either refuses it. The walk spends one of
  a budget of entries per entry seen, 500 000 by default; past it the repository is refused as
  too large to check, with a reason of its own, rather than checked in part.
- **Checked again before every command.** The tab's host is guarded (`Host::guarded`): before
  the load and before every command that reaches the repository, the directories and the
  alternates are checked again, without walking the git directory for links again, and a
  repository that now leads out is refused.
- **Working files.** Ocelli reads a working file itself only to hash it, as `git` does: a link as
  the text of its target, never the file it names, and nothing through a linked folder.
- **What leaves the server.** git's error texts never reach the browser: a failed command's
  reason and a load's status line say "Reading the repository failed" instead, and only
  Ocelli's own refusals pass as they are. A failed write says only that it was refused or failed
  and that a reload shows the repository as it is, `WRITE_FAILED`: the git adapter's refusals
  of a write cannot be told by their type from what git said. The sidebar's paths leave the
  server relative to the server root, or empty where they lie outside.
- **Submodules, worktrees, and the parent row.** A submodule or a worktree below the server root
  opens like any other repository, the same checks deciding; a submodule's git directory under
  the superproject's `.git/modules` passes where the superproject lies inside. Confined, a
  submodule's checked-out commit is read only where the submodule passes the check, and the
  superproject of the parent row is searched for only up to the server root and offered only
  where it passes the check. A row outside the server root is shown without its switch, the
  reason in its place.
- **No author on the working-directory node.** Confined, the node of uncommitted changes carries
  no author: the identity would come from configuration, which this confinement does not cover.
- **Choosing a repository.** In place of the desktop's folder dialog, the server lists the
  folders below a folder the browser names, resolved as an open resolves it, marking those that
  hold a `.git`, following no link out of the server root, and never showing or entering a part
  named `.git` in any case. It reads at most 10 000 entries and lists at most 1 000 folders,
  saying where there are more.

**Out of scope:** the configuration libgit2 reads for an open repository, the developer's global
and system configuration and the files a repository's own configuration includes through
`include.path` and `includeIf`, which [ADR-0035](0035-remotes-through-libgit2-transports.md)
and [ADR-0033](0033-writes-what-git-writes.md) govern; a link made inside a git directory after
the repository was opened, which the check before each command does not walk for; hard links,
which no check of a path sees; the desktop application and the terminal application, which open
what the developer picks; a list of several server roots.

## Alternatives considered

- **A `Confinement` value passed to `GitReader` and `GitWriter`** — explicit, no state shared by
  the process, and the tests could confine and not confine in one binary; but each read path
  would have to pass it on, and one that forgets opens unconfined, searching upwards out of the
  root. Process-wide, no read path can forget it, and a new one is confined without a line of its
  own. The cost is that a test that confines runs in a test binary of its own.
- **Any repository the user can read** — no root to name, as the desktop does; but a stolen
  session or a bug in the web interface then reaches every repository and every working directory
  the developer's account reads, the home directory's included.
- **A list of repositories named at start** — narrower still, but each new clone means a restart,
  and submodules and worktrees would have to be named one by one.
- **Checking the path the browser names, not the repository behind it** — catches `..` and links
  in the path, but not a `.git` file, a worktree, `core.worktree`, or an alternates file that
  leads outside, nor a search upwards that opens a repository enclosing the server root.
- **Following each link inside the git directory and checking where it leads** — keeps a
  repository with such a link openable, but the check would have to resolve each link as libgit2
  does for each file it reads; `git` writes no link there, so refusing costs no repository `git`
  made.
- **An unbounded walk** — checks every repository whole, but a git directory of millions of
  entries holds the blocking pool for as long as it takes; a refusal the developer can read is
  better than a check that never ends.
- **The full walk before every command** — would also see a link made after opening, at the cost
  of the whole walk per command on every repository; the cheaper check before each command covers
  the directories and the alternates, which a swap of `.git` changes.
- **Passing git's error texts to the browser** — they say most about what failed, but they name
  paths on the server, outside the root included.
- **The developer's home directory as a default root** — one argument less, but the default then
  reaches everything the alternatives above reach; naming the root is the developer saying what
  to show.

## Sources / Prior art

- libgit2 1.9.7, the version `libgit2-sys` 0.18.8 pins: `load_alternates` in
  `src/libgit2/odb.c` (lines split by `git__strtok` at `\r` and `\n`, `#` and empty lines
  skipped, a line starting with `.` joined to the objects directory, any other passed on as it
  stands, `GIT_ALTERNATES_MAX_DEPTH` 5), and `GIT_REPOSITORY_OPEN_NO_SEARCH` in
  `src/libgit2/repository.c`, which opens the folder given and searches no further, read in the
  copy `libgit2-sys` 0.18.8+1.9.7 vendors — <https://github.com/libgit2/libgit2>
- git2, `Repository::open_ext`, `path` (the git directory), `commondir`, and `workdir` —
  <https://docs.rs/git2/latest/git2/struct.Repository.html>
- `gitrepository-layout`: a `.git` file whose `gitdir:` points at the real repository, a
  `commondir`, `core.worktree`, and `objects/info/alternates`, the object stores a repository
  borrows from — <https://git-scm.com/docs/gitrepository-layout>
- `git-config`, `include.path` and `includeIf`, which read configuration files from anywhere —
  <https://git-scm.com/docs/git-config#_includes>
- Jupyter Server's `root_dir`, the folder the server serves files from — recalled, not verified
  for this record.
- OWASP, *Path Traversal* — <https://owasp.org/www-community/attacks/Path_Traversal> (recalled,
  not re-read for this record): resolving a path before comparing it with the allowed root.

## Consequences

- Positive: a browser that gets in reaches the folder the developer named and nothing beyond it,
  however a repository inside points outside; this narrows what a breach of Q-5 costs on the
  machine the server runs on.
- Positive: every open of the adapter is confined, the ones written later included, without
  passing anything through the read paths.
- Negative / trade-offs: a worktree below the server root of a repository outside it, a clone
  borrowing objects from outside, a repository with a link in its git directory, and one whose
  git directory holds more entries than the walk's budget do not open; the developer starts the
  server with a wider root, or fixes the repository.
- Negative / trade-offs: a link made inside the git directory after the repository opened is
  read through until the next open, and a hard link is never seen; the window is the developer's
  own account's to exploit, which other users of the machine cannot write into.
- Negative / trade-offs: the configuration libgit2 reads may come from outside the root; the
  working-directory node shows no author in the browser, so that none of it is shown.
- Negative / trade-offs: a test that confines the adapter runs in a test binary of its own, and
  the server's tests share one confined process with a root per test.
- Follow-ups: how the configuration libgit2 reads is confined, should a value of it ever be shown
  in the browser.

## Enforcement

Tests that cite this ADR with a `Verifies:` marker:

- in `crates/git/tests/confined.rs`, a binary of its own: confined, no read searches upwards out
  of the folder it is given; and in `crates/git/src/confine.rs`, that alternates are read as
  libgit2 reads them;
- in `crates/server/tests/server.rs`, on scratch trees with a server root, repositories outside
  it, and links between them: a path outside the root, a repository that reaches outside it, a
  link inside the git directory or an alternates line read otherwise, and each way out are
  refused, each with its reason; a repository swapped for a way out after it opened is checked
  again; a git directory larger than the walk is refused as too large; a linked folder or a path
  leading out reads nothing outside; what git says about a failed read reaches the browser only
  as "Reading the repository failed"; the sidebar names no path outside the root and reads no
  superproject or submodule outside it; a submodule and a worktree below the root open through
  their rows and lead back; and the browser chooses a repository from the folders below the root;
- in `crates/server/src/`, the unit tests of `root.rs` (opening, refusing, the listing and its
  bound, no `.git` shown or entered in any case), of `socket/scrub.rs` (the sidebar's paths and the
  scrubbed reasons), and of `lib.rs` (the server root is required);
- in `crates/commands`, that a guarded host checks the repository again before every command
  that reaches it;
- in `src-web`, that the way up the folder listing stops at the server root and that a row
  outside it shows without its switch.
