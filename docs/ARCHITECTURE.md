# Architecture — Ocelli

> The system **as it stands today**: the parts it is built from, what each is responsible for, and
> how they fit together. It holds no rule and no decision. It names the ADR behind each structural
> choice; where it disagrees with an accepted ADR, the ADR is right and this document is stale. It
> is updated in the same change as the structure it describes
> ([ADR-0001](adr/0001-agent-governance-model.md)).
>
> **Last design revision:** 2026-10-09, due after 20 changes. The revision that ran moves the
> date; the number is this project's to set; a sensor counts the changes outside `docs/` since
> the commit that set the date and says when the next is due
> ([ADR-0004](adr/0004-feature-layer.md)).

## Context

What sits outside the system and what crosses its boundary. A diagram earns its place here more
than anywhere else, and none beats one that has stopped being true.

```
 developer ──▶ desktop application (src-tauri + src-web) ─┐
 browser ═WS═▶ remote server (crates/server + src-web) ───┤──▶ ocelli-commands
 developer ──▶ terminal application (crates/tui) ─────────┤──▶ ocelli-core ◀── ocelli-git ──▶ repository
                                                          │    (layout, history,  (git2)     on disk
                                                          │     port)
                                                          └─ all depend on ocelli-core and ocelli-git,
                                                             the two hosts of the web interface
                                                             through ocelli-commands
```

Ocelli reads repositories on the local file system through git2, and writes to them through git2
only what the developer asks for, a tag, a stash, the index, a commit, a branch, a merge, a
rebase, a cherry-pick, a revert, a reset, and the resolution of their conflicts
([ADR-0031](adr/0031-writes-what-git-writes.md)).
It reaches a remote only when the developer fetches, pulls, or pushes, through libgit2's own
transports with the credentials of the developer's own setup
([ADR-0034](adr/0034-remotes-through-libgit2-transports.md)),
and makes no other network request but Gravatar avatars in the desktop application, off until the
developer turns them on ([ADR-0025](adr/0025-no-egress-hardened-webview.md)). The web interface runs
inside the desktop host, or in a browser served by the remote server, which listens on the
loopback addresses of the machine holding the repositories and answers only a browser that
logged in with the connection token it printed
([ADR-0036](adr/0036-remote-server-localhost-tls-session-cookie.md)),
and only about the repositories below its server root
([ADR-0037](adr/0037-git-adapter-confined-to-the-server-root.md));
it only reads unless started with `--allow-writes`, and then writes as the user it runs as
([ADR-0035](adr/0035-remote-server-websocket-per-tab.md)).
The
measurement of the quality goals, which ships to nobody, is the one part that runs the git binary
and fetches over the network: the corpus of public repositories, in the Measure workflow.

## Building blocks

The parts the system is made of, each with one responsibility, named in the vocabulary of
[`GLOSSARY.md`](GLOSSARY.md). One level deep, deeper only where the size of a part earns it. The
golden path and the test pattern an agent copies from are named here.

- **Core** (`crates/core`, `ocelli-core`) — the commit graph's model, its layout, and the repository
  ports, `RepositoryReader` and `RepositoryWriter` with the `Write`s it takes and `perform`, which
  every front end writes through, handing on a transfer's progress and the developer's stop, and
  which refuses a write that discards work without the developer's confirmation
  ([ADR-0033](adr/0033-discarding-work-only-after-confirmation.md)). It depends on no technology
  and no adapter ([ADR-0008](adr/0008-ports-and-adapters.md)); the layout in `graph.rs` is pure
  functions over values ([ADR-0012](adr/0012-functional-core-layout.md)), laid out after Pvigier
  ([ADR-0013](adr/0013-layout-after-pvigier-by-chunk.md)) with the main line alone in the
  leftmost lane ([ADR-0014](adr/0014-main-line-in-the-leftmost-lane.md)) and edges shaped by type, a
  merge out of the main line up a track of its own
  ([ADR-0015](adr/0015-edge-shapes-and-tracks-out-of-the-main-line.md)). `History` in `history.rs`
  keeps the graph laid out so far and answers the front ends' reads of it: a window of rows with the
  edges crossing it, search, and lineage ([ADR-0019](adr/0019-front-ends-read-row-windows.md)).
  `transfer.rs` decides the fetch, pull, and push of the checked-out branch the refs make possible,
  or why there is none, and which transfer can be stopped, and `operations.rs` which operations of
  G-13 a commit offers on the checked-out branch, for both front ends to draw; the shared `Host`
  of `ocelli-commands` sends these answers to the web interface in both hosts, which restates
  none of them
  ([ADR-0022](adr/0022-desktop-host-answers-what-the-core-decides.md)).
  `load.rs` runs a load session through the port, and a `LoadSession` is what a front end keeps of
  it, its `History` and its status line, for both front ends and the measurement alike.
- **Git adapter** (`crates/git`, `ocelli-git`) — `GitReader`, the repository port implemented with
  git2: the main line's tip, the history in chunks of 3 000 commits, the first with the stash and
  working-directory nodes, the refs the sidebar lists with whether each local branch is merged,
  changed files, the working-directory node's with how much of each is staged and the content shown,
  and diffs ([ADR-0017](adr/0017-git2-repository-reader.md),
  [ADR-0016](adr/0016-history-loads-in-chunks.md),
  [ADR-0018](adr/0018-synthetic-nodes-in-the-first-chunk.md)); the walk runs in commit-date order
  and turns libgit2's object cache off for the whole process. `operations.rs` holds the history
  operations of G-13 from a clean tree, stopping on a conflict in git's own state, and
  `conflicts.rs` resolves such a conflict file by file and continues or aborts the operation;
  the refs carry the operation in progress with its conflicts, read in `refs.rs` from git's
  state whoever started it. `GitWriter` implements the writer port in `write.rs`, which
  dispatches each write and holds the tag, index, commit, and branch writes, the stash's in
  `stash.rs`, each checked against the state it names before anything changes by the checks of
  `guard.rs`, the index and the working directory written through `checkout.rs`, every libgit2
  call that writes the working directory made in `workdir.rs` behind one guard against a link in
  the way, a malformed tree a hostile remote delivered, or a name the file system cannot hold,
  and giving a conflict's sides the path-safe labels `git` writes, so libgit2 writes a side
  beside its path, and a message
  cleaned up by `message.rs` as `git` cleans it up, and returns the commit a write made; a
  commit or an annotated tag is refused where the configuration asks for a signature. A path a
  caller gives, to stage, unstage, resolve, or read a diff, is refused before git2 sees it unless
  it is plain names below the working directory outside any `.git`, and the index is read at a
  path only through `git::index_entry`, since git2's `Index::get_path` panics on some; a write any
  string of which holds a NUL is refused before git2 sees it, and a reference name is checked
  only through `guard::valid_ref_name`, since git2 unwraps both into C strings. Each write
  does what `git` does for the same command, and where it departs, the writer's ADRs list why
  ([ADR-0031](adr/0031-writes-what-git-writes.md), and
  [ADR-0032](adr/0032-conflict-labels-path-safe.md) for a conflict's labels).
  A fetch, a pull, which then fast-forwards the checked-out branch, and a push, never by force
  and where `push.default` sends it, live in `remote.rs` and reach the remote through libgit2's
  own transports with the SSH agent and the credential helper of the developer's global and
  system configuration, a helper asked only for a remote named by its domain over HTTPS, and
  with libgit2's server timeouts set once for the process, the git adapter's one `unsafe`
  block; a push learns what the remote took from the remote-tracking branch libgit2 moves, or,
  where it moves none, from the remote's branch fetched again from the address pushed to into a
  scratch reference under `refs/ocelli/`, which no load and no digest of the refs counts
  ([ADR-0034](adr/0034-remotes-through-libgit2-transports.md)).
  The refs name the default remote; for the remote server's turns, `refs.rs` also reads a
  repository's resolved common directory (`common_dir`) and a digest of where its refs point
  (`refs_digest`). Neither `GitReader` nor `GitWriter` starts a program a repository names, a
  hook included (Q-4). `confine.rs` decides for the remote server whether a repository
  opened without searching upwards lies wholly below its server root: its git, common, and
  working directories and every object store its alternates borrow, read as libgit2 reads them,
  with no link anywhere inside its git and common directories, a walk bounded by a count of
  entries past which the repository is refused as too large to check; a link made there after
  the repository was opened, and a hard link, are not seen. Once the remote server confines the
  process to its root, every read opens the folder it is given without searching upwards, the
  parent row is looked for only up to the root, a submodule is read only where it lies below
  it, and the working-directory node carries no author, which would come from configuration;
  the desktop and terminal applications never confine it. The configuration libgit2 reads, the
  developer's global and system files and what a repository's own includes with `include.path`
  and `includeIf`, lies outside what ADR-0037 confines. Ocelli reads a working file itself only
  to hash it for staging (`working_blob` in `git.rs`), as `git` does: a link as its target's
  text, and nothing through a linked folder
  ([ADR-0037](adr/0037-git-adapter-confined-to-the-server-root.md)).
- **Terminal application** (`crates/tui`, `ocelli-cli`) — draws the core's graph with box-drawing
  characters on ratatui; it loads and lays out the history chunk by chunk on a loader thread, keeps
  the core's `History` at its event loop, reads files and diffs through `GitReader` on a worker
  thread, writes through `GitWriter` on a thread of its own, one write at a time, which
  `writes.rs` holds with the requests the keys build, as `writes.ts` does for the web interface,
  showing a
  transfer's progress in the status line and stopping a fetch or a pull on `Esc`, lists an
  operation in progress with its conflicts, resolved, continued, or aborted from there, and keeps
  its view state in a file
  ([ADR-0029](adr/0029-terminal-application-ratatui.md), [ADR-0030](adr/0030-cli-state-file.md)).
- **Measurement** (`crates/measure`, `ocelli-measure`) — a command that lays out a repository as
  the front ends do and measures a quality goal on it: the number of edge crossings for Q-1, next
  to those of `git log --graph` over the same commits, and the waits of Q-2: the first window of
  rows as soon as the first chunk is laid out, and every window across the history once all of it
  has loaded into the core's `History`. It drives the core through the git
  adapter and runs the git binary, with the repository's configuration kept from choosing a
  program; it is not shipped to developers, and Ocelli itself never runs git (Q-4). The memory of
  the desktop application for Q-3 is measured from outside, by `scripts/measure-memory.sh`: it
  starts the application with no repository and on a generated one, waits there for the status
  line the desktop host writes to standard error once the graph has loaded, sums the memory of the
  application's processes, the WebView's included, and takes the difference as what the graph
  adds. `scripts/measure-crossings.sh` runs the command on a fixed corpus against recorded counts.
  For Q-2, `scripts/measure-windows.sh` runs its `windows` measurement on a generated history of a
  million commits with thirty branch lines side by side and some dates running backwards: the wait
  for the first window of rows, for the slowest chunk taken in while the rest loads, and for the
  slowest window across the loaded history; `scripts/generate-history.sh` writes the generated
  histories of Q-2 and Q-3. `scripts/measure-mutants.sh` changes the git adapter's guards in
  `guard.rs`, `remote.rs`, and `confine.rs`, and the turns' `common_dir` and `refs_digest` in
  `refs.rs`, one mutant at a time and lists each one the suites of
  the git adapter and the remote server let through. All four run weekly and on demand in
  `.github/workflows/measure.yml`, not in the checks
  ([ADR-0007](adr/0007-toolchain-rust-and-typescript.md)); the checks run only their self-tests.
- **Host commands** (`crates/commands`, `ocelli-commands`) — the commands and events of a host of
  the web interface, once for the desktop host and the remote server, free of any transport
  ([ADR-0035](adr/0035-remote-server-websocket-per-tab.md)).
  A `Host` holds what one window or tab has open: it loads the history through `GitReader` on
  Tokio's blocking pool into the `History` of a load session (`session.rs`), answers the web
  interface's reads of it, windows of rows, search, and the selected commit with its lineage,
  and writes through `GitWriter` on the same pool, one write at a time, telling the web interface
  a transfer's progress as `write-progress` events at most every tenth of a second and stopping it
  on `cancel_write` ([ADR-0019](adr/0019-front-ends-read-row-windows.md) for the reads,
  [ADR-0035](adr/0035-remote-server-websocket-per-tab.md) for the writes); a `Host` given
  a `TakeTurn` writes only once it gives it the repository's turn, and tells the turn how the write
  ended before giving it back. Its events go to a
  `Report`, the window or the socket. A command fails with a `HostError`: no repository open, a
  refusal with its reason, or what git said, the kind from which the remote server decides what
  the browser reads. `COMMANDS` lists the commands once, each with how the two hosts answer it,
  which the remote server's tests hold the desktop host's handlers, the server's dispatch, and
  what the web interface sends to. Once a load ends it hands the memory the load freed back to
  the system through glibc's `malloc_trim`, one of the workspace's two `unsafe` blocks outside
  its tests, libgit2's timeouts the other.
- **Desktop host** (`src-tauri`, `ocelli`) — the Tauri shell: its commands hand their arguments
  to the shared `Host` and its events to the window, and it adds the native folder dialog
  ([ADR-0021](adr/0021-tauri-desktop-host.md)).
- **Remote server** (`crates/server`, `ocelli-server`) — serves the web interface's build,
  embedded in the binary by `build.rs`, and carries the desktop host's commands and events with
  the same names and payloads over one WebSocket per browser tab, each connection a shared
  `Host` of its own whose load stops when the connection closes, or when it does not read the
  bounded queue of what it is sent in time (`socket.rs`), and whose load session `Sessions`
  counts until the connection, its commands, and its load have let go; it links no part of
  Tauri or WebKit
  ([ADR-0035](adr/0035-remote-server-websocket-per-tab.md)).
  `auth.rs` holds the connection token drawn at each start, the session ids it is exchanged
  for, a bounded number with the oldest giving way, and the count of wrong tokens, past a few of
  which every wrong token's answer waits, doubling up to a cap, one at a time for the whole
  server, while the right one is answered at once; it decides which `Host` and `Origin` a
  request may carry. `lib.rs` confines the git adapter to the root at start, serves a bounded
  number of connections and of tabs at once, each connection through hyper with a timeout on
  reading a request's headers, refuses every request without a valid session cookie
  before it serves a file or reads a command, and sets the Content-Security-Policy, with the
  hash of `index.html`'s inline script, and the referrer policy on every answer
  ([ADR-0036](adr/0036-remote-server-localhost-tls-session-cookie.md)).
  `root.rs` resolves a path from the browser against the server root and opens a repository
  only where the git adapter's `confine` finds it wholly below the root, on the blocking pool,
  and the tab's `Host` checks its directories and alternates again on the blocking pool before
  every command that reaches it. Every event, and the failure of every command that reached the
  repository, leaves the server through `socket/scrub.rs`: the sidebar's paths leave the server
  relative to the root, or empty where they lie outside, which
  the web interface's `repoSwitch.ts` shows as a row without its switch and says why, and
  whatever git says about a failed read reaches the browser only as "Reading the repository
  failed"
  ([ADR-0037](adr/0037-git-adapter-confined-to-the-server-root.md)).
  Started without `--allow-writes` it refuses every write with a reason the web interface shows;
  with it, a tab's `perform_write` and `cancel_write` reach its `Host` as the desktop's do, as the
  user the server runs as, a transfer with the SSH agent of the server's environment and the
  credential helpers of its account's global and system configuration, to a remote over the network
  or one on the machine below the root, which the confined git adapter decides with
  `confine::remote_within_root` before libgit2's local transport opens it, its `write-progress` sent
  to the tab that started it with an address's user and password and any path on the server scrubbed
  (`scrub_progress`), and what git says about a write reaching the browser only as that it was
  refused or failed. `turns.rs` gives a write its repository's turn, keyed by the resolved common
  directory the git adapter's `refs` reads, which the `Host` takes after the root's check and
  before checking the root again and writing, so that two tabs' writes to one repository run one
  after the other, a bounded number waiting for each repository, one past them refused; when a
  connection ends its commands are aborted and a write of its still waiting for its turn gives it up
  unwritten. Its registry of the tabs, `turns/tabs.rs`, keeps the repository each tab has open by
  its common directory: before
  the turn is given back, a write done, or refused after it moved a ref, tells every other tab on
  the repository, through a worktree of it included, to load again, with the server's own
  `repo-written` event, which `SERVER_EVENTS` of `ocelli-commands` lists and the desktop host never
  sends
  ([ADR-0035](adr/0035-remote-server-websocket-per-tab.md)).
  It answers in place of the
  desktop's folder dialog `list_folders`, the folders below a folder of the root, those
  holding a `.git` marked, no link out of the root followed, a bounded number read. With
  `--listen` it binds that address alone and speaks TLS alone through rustls with ring
  (`tls.rs`): the developer's certificate and key, or a self-signed one rcgen makes on the
  first start and keeps with its key in the user's configuration directory, readable by the
  account alone, reused and refused rather than replaced where it does not name the address, or
  the host name for an unspecified one, where half of it is kept, or where its key is readable
  by others; its fingerprint is printed, a `Host` is admitted only where the certificate names
  it, the origin is `https`, and the session cookie `Secure` with the `__Host-` prefix. HTTP
  and the WebSocket run on axum and hyper over Tokio.
- **Web interface** (`src-web`) — Svelte and TypeScript: the canvas, which holds only the window of
  rows it draws, the sidebar, the commit detail with its tag and branch forms, its stash actions,
  its ref chips' switch and delete, its merge, rebase, cherry-pick, revert, and reset onto the
  checked-out branch, and the working-directory node's stage toggles and commit form,
  the diff overlay, the search field, the banner of an operation in progress with its conflicted
  files, the dialogs that describe a stash and confirm a write that discards work, the toolbar's
  Fetch, Pull, and Push, which give way to a Cancel while a fetch or a pull runs, and themes;
  `writes.ts` holds the writes it hands the host, one at a time; the toolbar draws the transfers
  the host sends with the refs and the commit detail the history operations it sends with the
  selection, and `refChips.ts` decides which ref each chip switches to or deletes. It reaches
  its host only through `host.ts`, which picks at start-up Tauri's `invoke` and `listen` inside
  the desktop host's WebView or else the remote server's WebSocket, whose `SocketTransport` in
  `transport.ts` matches each answer to its command by id and hands events to their listeners
  ([ADR-0035](adr/0035-remote-server-websocket-per-tab.md),
  [ADR-0023](adr/0023-svelte-web-interface.md), [ADR-0020](adr/0020-virtual-rendering.md),
  [ADR-0026](adr/0026-diff-overlay-and-file-list.md),
  [ADR-0024](adr/0024-themes-tokens-and-palettes.md), [ADR-0027](adr/0027-repository-sidebar.md)).

**The golden path** is the core's chunked layout and its `History` behind the git adapter's
chunked load: `crates/core/src/graph.rs` and `crates/core/src/history.rs`, fed by `load_chunks`
in `crates/git/src/lib.rs`. **The test pattern** is a reference test: a fixed or generated
input and the output the reference implementation produced, compared exactly
(`crates/core/tests/layout_reference.rs`, `crates/git/tests/load_reference.rs`), beside the
structural tests of the shape in `crates/core/tests/structure.rs`. A write is tested on a
repository `git` made, and what it did is read back with `git` (`crates/git/tests/write.rs`,
`crates/git/tests/history.rs`); a refused write goes through `refused`, which compares one
snapshot of the repository before and after. Each local write also runs beside the `git` command
it stands for, on two copies of a repository in every state that changes its outcome
(`crates/git/tests/git_parity.rs`), each departure listed with the ADR that decides it, and the
two copies' snapshots compared with the commits made since they parted read by content. The
scratch directory, the test's `git`, the snapshot, and `refused` are written once, in
`crates/git/tests/support/mod.rs`; every test starts `git` through
`crates/git/tests/git_command/mod.rs`. A way out of the server root is tested in
`crates/git/tests/confine_checks.rs`, each case built so that the clause of `confine.rs` it is
about decides it; the remote server's tests keep one case of each kind at the WebSocket. The two
reads of `refs.rs` the remote server's turns take are tested in `crates/git/tests/turn_reads.rs`.
The remote server's tests open a tab as a browser does through
`crates/server/tests/browser/mod.rs`, whose `Pair` makes each write in the tab and through the
shared `Host` on a twin of the repository, as the desktop host does, and compares the two
(`crates/server/tests/writes.rs`, and `crates/server/tests/transfers.rs`, a binary of its own for
the developer's configuration it sets for the process, with a remote below the root and a local
server that stalls a fetch). Each concern of the server is a test binary of its own:
`server.rs` holds its reads, its login, its bounds, its confinement to the root, and that it
links no part of Tauri or WebKit; `allow_writes.rs` what `--allow-writes` lets through and holds
back, the turns of a repository's writes, and the programs a repository names, none of which its
writes run while `git` performing the same writes runs each (Q-4);
`command_list.rs` the shared list of commands each host answers; `tls.rs` where it listens and
its TLS; `hostile_commands.rs` every command sent hostile arguments.
Where the web interface spells the JSON of a core value,
the core's `tests/write_fixture.rs` writes an example to `crates/core/tests/fixtures/` and the web
interface's test reads it against its types. The server's own `repo-written` event is held to
the web interface's types by `crates/server/tests/fixtures/repo-written.json`, which the server's
test writes and `transport.test.ts` reads. `crates/git/tests/hostile_writes.rs` writes every
kind of write with each string field set to each hostile value, and none may panic;
`crates/git/tests/confined_remotes.rs`, a binary of its own that confines the process, reaches a
remote on this machine.

## How it runs

The few paths worth following end to end, such as a request, a job, or a build, and where state
lives between them. Only what a newcomer would otherwise reconstruct from code.

- **Opening a repository.** A front end's host runs the core's `load` on a thread of its own: it
  asks `GitReader` for the main line's tip, then for the history chunk by chunk, lays each chunk out
  with a `Layout` that keeps the leftmost lane for that tip's main line
  ([ADR-0014](adr/0014-main-line-in-the-leftmost-lane.md)), reads the refs once the first chunk is
  handed over, and hands progress, chunks, refs, and the end over as
  events, each chunk to be appended to a `LoadSession`. The terminal application receives the
  events at its event loop as messages and reads the window of rows in view from its session. The
  shared `Host` of `ocelli-commands`, in the desktop host and in each tab of the remote server,
  runs the load on Tokio's blocking pool, appends the chunks to the `LoadSession` of
  its current load, and tells the web interface as `history-grew` and `load-refs` events, the
  refs with the transfers the core decides from them; the web
  interface then reads the window of rows in view, searches, and selects through commands, and a
  newer load refuses an older one's chunks, which stops its walk. A path on the command line opens
  the same way: `ocelli-cli [PATH]` from the start, and the desktop host hands its path to the web
  interface through the `startup_repo` command once the interface's listeners stand, so the load
  runs as one the developer started.
- **Opening a repository in a browser.** `ocelli-server --root <folder>` prints a URL with its
  connection token; the browser's first request exchanges it for a session cookie and is
  redirected to the URL without it, the web interface is served, and it opens a WebSocket to
  `/ws` from the server's own origin. Its `startup_repo` answers the server root, `.`, where the
  root is a repository, and `open_repo` takes paths relative to the root, which the web
  interface's `FolderDialog` lets the developer choose from `list_folders` where the desktop
  shows its folder dialog; the load then runs as in the desktop host, its events going to that
  tab's socket.
- **Selecting a commit.** The core computes the commit's lineage, which the windows read after
  light up, and the history operations it offers, which the `Host` answers with the
  selection once its load's refs are read, and the front end asks `GitReader` for the changed
  files, and for the diff of a file once one is opened; the working-directory node's sentinel SHA
  reads the uncommitted
  changes, each file with how much of it is staged and the content shown, which staging expects.
- **Writing.** The front end hands the core's `Write` to `port::perform` off the thread that
  draws, which passes it to `GitWriter`; a write that discards work, as `Write::discards` names
  it, passes only with the developer's confirmation, which the front end asked for in words that
  name what is lost ([ADR-0033](adr/0033-discarding-work-only-after-confirmation.md)); the web
  interface sends every write as the core's `Write` in JSON, asking the `Host`'s
  `write_discards` what it discards and handing it to `perform_write`, which the remote server
  performs only where it was started with `--allow-writes`. A write refused changes
  nothing and its reason goes to the developer. A write done starts a new load session of the
  same repository, which takes the commit the write made, or else the commit selected before, and
  names its row once a chunk lays it out, unless the developer has chosen another by then; the
  shared `Host` passes the row on with `history-grew`, and the front end selects it
  ([ADR-0031](adr/0031-writes-what-git-writes.md)).
- **Transferring.** A fetch, a pull, or a push is a `Write` like any other, handed to
  `port::perform` with a callback that the git adapter tells how far the transfer has come and
  that answers whether to go on; the front end shows the progress in its status line, and a stop
  the developer asks for ends a fetch, or a pull while it fetches, at libgit2's next callback. A
  push has no stop: libgit2 cannot end one once its objects are sent. A transfer done reloads as
  every write does; a refused one leaves the view as it was, the remote branches a pull's fetch
  moved before its refusal included, until the next reload
  ([ADR-0034](adr/0034-remotes-through-libgit2-transports.md)).
- **State between sessions.** The web interface keeps the theme, the avatars toggle, the diff
  layout, and the sidebar in `localStorage`; the terminal application keeps the same but the
  avatars toggle in `ocelli/cli-state.json` under the user's state directory.
