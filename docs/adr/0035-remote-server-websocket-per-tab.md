# ADR-0035: The remote server carries the desktop host's commands and events over one WebSocket per tab, and writes only when started with `--allow-writes`, as the user it runs as, one write per repository at a time

- **Status:** 🟢 accepted
- **Date:** 2026-10-10
- **Deciders:** Markus Brigl
- **Applies to:** the remote server's crate and binary `ocelli-server`, the command handling the desktop host in `src-tauri/` and the remote server share in `crates/commands`, the transport through which `src-web/` reaches either host, the fixtures of the wire format in `crates/core/tests/fixtures/` and `crates/server/tests/fixtures/`, the `--allow-writes` switch, how `crates/server/src/socket.rs` answers `write_discards`, `perform_write`, and `cancel_write` and what it does when a tab closes, the turns of a repository's writes and the registry of open repositories in `crates/server/src/turns.rs`, and the tests of Q-4, Q-5, Q-7, and Q-6 against the remote server

## Context

G-18 asks for G-1 to G-17, the commit graph and its operations, in three front ends, the web
interface served by the remote server among them, the writes of G-12 to G-16 included; G-17 asks
that a fetch, pull, or push reports its progress and that a fetch, and a pull while it fetches,
can be stopped. The Constraints describe the remote server as VS Code's remote development does:
it runs next to the repository, and the developer's browser talks to it. The desktop host answers
the web interface through Tauri commands and events ([ADR-0021](0021-tauri-desktop-host.md)): the
core keeps the history in the host and the web interface reads windows of rows, searches, and
selects ([ADR-0019](0019-front-ends-read-row-windows.md)), the host sends the answers the core
decides from the refs ([ADR-0022](0022-desktop-host-answers-what-the-core-decides.md)), and writes
go through the core's port ([ADR-0031](0031-writes-what-git-writes.md)).
[ADR-0007](0007-toolchain-rust-and-typescript.md) writes the servers in Rust, and
[ADR-0008](0008-ports-and-adapters.md) makes every front end an adapter of `core`.

A browser can open the same server in several tabs. The web interface asks one command at a time
and listens to events that arrive while a load or a transfer runs. Q-2 holds for every front
end: the first screen within 2 s, scrolling never waiting more than 100 ms for the next. Q-3 is
measured on the desktop application only. A server that pulls in WebKit or Tauri carries a
desktop's system libraries onto a machine that has no display.

Writes from two tabs to one repository have to be ordered. That makes the tabs share more than
the process: a turn for each repository's writes, the knowledge of which tab has which
repository open, and an event that tells a tab another one wrote to it. The fixture of that
event lies beside the server's tests, not among the core's.

The writes, their checks, and their confirmations exist once: the shared `Host` in
`crates/commands` performs a write through the core's port, asks `write_discards` for the words
of the question the web interface puts before a write that discards work, and accepts such a
write only with `confirmed` ([ADR-0033](0033-discarding-work-only-after-confirmation.md)). A
transfer reaches remotes with the running SSH agent and the credential helpers of the global and
system configuration only ([ADR-0034](0034-remotes-through-libgit2-transports.md)), reports
`write-progress`, and stops on `cancel_write`. After a write the web interface loads again and
selects the commit made or the one selected before (ADR-0031). Each `Host` refuses a second write
while its own first runs.

A write waiting for another tab's holds a thread of the blocking pool, and a tab can close while
its write waits or its fetch runs. Q-5 says the server hands control of a machine to whoever
reaches it: authentication ([ADR-0036](0036-remote-server-localhost-tls-session-cookie.md))
and the server root ([ADR-0037](0037-git-adapter-confined-to-the-server-root.md)) narrow
what a stolen session reaches, and a server that writes widens what it can do there: a commit,
a reset, a push to a remote with the developer's credentials. Q-4 forbids running a
program a repository names, Q-7 discarding work without confirmation, Q-6 a repository obtaining
the developer's credentials. The server serves one developer (Non-Goals) and runs under their
account, as VS Code's remote server does.

## Decision

We will build the remote server as a binary of its own, `ocelli-server`, free of Tauri and
WebKit, serving the built web interface over HTTP and carrying the desktop host's commands and
events, with the same names and payloads, over one WebSocket per browser tab, each tab a load
session of its own; the tabs share the process and, where it writes, the turns of each
repository's writes and which repository each has open. It performs the desktop host's writes and
transfers only when started with `--allow-writes`, as the user it runs as, through the same
commands and confirmations, one write per repository at a time.

- **A binary of its own.** `ocelli-server` is a workspace crate that depends on `core`, the git
  adapter, and the shared command crate, and links no part of Tauri or WebKit, so it runs on a
  machine without a display.
- **One implementation of the commands.** The command bodies and the load session live in
  `crates/commands`, free of Tauri, which takes a sink for events; `src-tauri` and
  `ocelli-server` each adapt it to their transport and add nothing to what a command answers. A
  command a host cannot offer, the desktop's native folder dialog `browse_repo`, has a
  counterpart in the other host, never a different answer to the same name.
- **The wire format.** A message from the browser is a command with an id and its arguments, as
  `invoke` passes them; the server answers with the same id and either the value the Tauri
  command returns or the error it rejects with. An event is its name and the payload the host
  emits. All of it is JSON, the payloads held to the web interface's types by fixtures: the
  core's in `crates/core/tests/fixtures/`
  ([ADR-0022](0022-desktop-host-answers-what-the-core-decides.md)), and the server's own event's
  in `crates/server/tests/fixtures/`.
- **A transport in the web interface.** `src-web` reaches its host only through a transport of
  its own with two implementations, Tauri's `invoke` and `listen`, and the WebSocket; the build
  served by `ocelli-server` and the build the desktop host shows differ in that choice alone.
- **What it serves.** The files of the web interface's build, embedded in the binary, with the
  headers [ADR-0025](0025-no-egress-hardened-webview.md) sets for the WebView as HTTP headers:
  a Content-Security-Policy that permits the bundle's own scripts and styles, a connection to the
  server itself, and only `img-src https://www.gravatar.com` beyond it; avatars stay off until the
  developer turns them on in the web interface, as in the desktop application.
- **A load session per tab.** Each WebSocket connection holds what the desktop host holds for its
  window: its repository, its load session, its selection, and its write in flight. A bounded
  number of tabs is open at once, a handshake past them refused. When a connection closes, its
  load stops and its session is dropped, and a tab that reconnects opens its repository again.
- **What the tabs share.** Besides the process, and only where the server was started with
  `--allow-writes`: the turn of each repository's writes, the registry of the repository each tab
  has open, keyed by its canonical common directory, and the server's own `repo-written` event,
  through which one tab's write reaches the others. Nothing of one tab's load session, selection,
  or write in flight reaches another.
- **The switch.** Without `--allow-writes` the server refuses every `perform_write` with a reason
  the web interface shows, and changes nothing: a session taken over reads but writes nothing.
  With it, the server answers every write the desktop host answers. The switch holds for the
  whole run; there is no way to turn writes on or off while the server runs.
- **As the user it runs as.** A write records the identity of the repository's configuration as
  the server's account reads it (ADR-0031). A fetch, pull, or push authenticates with the SSH
  agent of the server's environment and the credential helpers of that account's global and
  system configuration, never the repository's (ADR-0034, Q-6). The browser hands over no
  identity, key, password, or helper, and the server asks it for none.
- **The desktop host's commands.** `write_discards`, then `perform_write` with `confirmed`, cross
  the WebSocket with the names and payloads the desktop host's Tauri commands carry; the shared
  `Host` decides which write discards work and refuses one that comes without its confirmation,
  as in the desktop. The server adds no confirmation of its own, no token per write, and no write
  logic of its own.
- **Progress and stop.** A transfer reports its progress to the tab that started it as
  `write-progress` events; `cancel_write` from that tab stops its fetch, or its pull while it
  fetches (G-17). A push offers no stop, as in the desktop.
- **One write per repository.** The process holds a turn for each repository, keyed by its
  canonical common directory, so that a worktree and the working directory it belongs to count as
  one. A tab's write waits for a write of another tab to the same repository to end, and then
  checks again what it expects (ADR-0031), refused with nothing changed where the first changed
  it. At most 8 writes wait for one repository's turn; one past them is refused at once with a
  reason the web interface shows, since each waiting write holds a thread. A tab still runs one
  write of its own at a time, a second refused while its first runs.
- **A closed tab.** When a tab's connection closes, a write of its still waiting for its turn
  gives the wait up and never writes; its fetch, or its pull while it fetches, stops as
  `cancel_write` stops it, so that the repository's turn is not held for a transfer nobody waits
  for; a local write already writing, and a push, finish, as a write in the desktop host does when
  its window closes. The tab leaves the registry, and no other tab's write tells it anything.
- **Every tab reloads.** After a write, done or refused or failed after it moved a ref or `HEAD`,
  the server tells every other tab with that repository open, through a worktree of it included,
  with `repo-written`, before the turn is given back; that tab loads again as the writing tab
  does. The desktop host, with one window per repository it shows, never sends it.
- **The root.** Every write passes the check before every command that reaches the repository
  (`Host::guarded`, ADR-0037), and the confined git adapter opens it with `NO_SEARCH`, writes and
  transfers included. A fetch or a push reaches a remote on this machine, one libgit2's local
  transport opens in-process, named by a path or a `file://` address, only where its address,
  `insteadOf` and `pushInsteadOf` applied, is an absolute path that resolves below the server
  root; one outside is refused before anything is fetched or pushed, since a fetch would copy
  its objects into a repository the browser reads and a push would write it. A remote over the
  network, by an ssh or an https address, is the server account's to reach, as it is `git`'s.

**Out of scope:** who may reach the server and how it authenticates (ADR-0036); which
repositories a browser may open (ADR-0037); the HTTP and WebSocket library, axum and hyper over
Tokio within ADR-0007; credentials entered in the browser; accounts, roles, or several developers
on one server; writes the desktop application does not offer; which tabs may write, all of them
may; a cap on how long a write waits for another; the memory each tab's load session holds; the
working-directory node's author, which stays absent in the browser (ADR-0037).

## Alternatives considered

- **HTTP for commands and Server-Sent Events for events** — plain requests a proxy and the
  browser's tools understand, but a load and a transfer then span two channels whose order the
  web interface has to rebuild, and every command pays a request's headers; one WebSocket keeps a
  command's answer and the events around it in the order the host sent them.
- **A mode of the desktop application** (`ocelli --serve`) — one binary to ship, but the server
  would link Tauri and WebKit and need a desktop's system libraries on a machine without a
  display, and a flag would decide whether the process listens on a network.
- **One session at a time** — the simplest state, but a second tab, or the same tab reloaded
  while a load runs, would take the first one's history from under it.
- **A protocol of its own, shaped for the network** — commands named and cut for a remote round
  trip, but then two hosts answer the same web interface differently, and every answer is written
  and tested twice; the desktop host's commands already read windows, not the whole history.
- **The server's own copy of the command bodies** — no shared crate to keep free of Tauri, but
  each command then lives twice, the duplication ADR-0022 keeps out of the web interface.
- **Serving the web interface from disk beside the binary** — a smaller binary, but a second
  artifact to install and a directory whose content decides what the browser runs.
- **Writes always on** — no switch to remember, but every server started to look at a history
  can then commit, reset, and push with the developer's credentials for whoever takes a session
  over; a server that only reads loses nothing to a stolen session but what it shows.
- **Identity and credentials entered in the browser** — the server's account would need no agent
  and no helper, but a password or key then crosses the network, lives in the browser and in the
  server's memory, and Ocelli stores credentials, which ADR-0034 rules out.
- **A confirmation token of the server's own for each write** — a second proof beside the session
  cookie, but whoever holds the session receives the token through the same socket; it guards
  nothing the cookie and the Origin check do not, and the desktop's confirmation already keeps
  work from being discarded by accident (Q-7).
- **One writing tab only** — no ordering to decide, but a developer with two tabs on one
  repository finds one of them read-only for no reason they can see, and a lost tab holds the
  write until the server notices; the turn and the check of what a write expects order two tabs
  as they order Ocelli and another tool.
- **Unbounded waiting writes** — no refusal to explain, but every waiting write holds a thread of
  the blocking pool, and a session that sends writes to a busy repository would exhaust it for
  every tab's reads.
- **A closed tab's writes and fetch run to their end** — the desktop's behaviour for a closed
  window, but a waiting write would then land after nobody can see it, and a long fetch would
  hold the repository's turn from the tabs still open; a local write already running and a push
  finish, since neither can be stopped safely.
- **Local writes only, without transfers** — no credentials touched on the server, but G-18 asks
  for fetch, pull, and push in the browser, and the developer would leave for a terminal on the
  server to do what the desktop does in a click.

## Sources / Prior art

- VS Code, `code serve-web`: a CLI command that serves the web build of the editor from one
  process, by default on `localhost`
  (`cli/src/commands/args.rs`, `ServeWebArgs`, read 2026-10-08) —
  <https://github.com/microsoft/vscode/blob/main/cli/src/commands/args.rs>
- RFC 6455, *The WebSocket Protocol* — <https://www.rfc-editor.org/rfc/rfc6455>
- Tauri, *Calling Rust from the frontend* and *Calling the frontend from Rust*: commands with
  arguments and a result or error, events with a payload —
  <https://v2.tauri.app/develop/calling-rust/>
- VS Code Remote Development: the remote server runs as the user who started it, its source
  control writes with that user's git configuration and credentials on the remote machine, and
  the local side forwards the SSH agent only where the developer asks — recalled, not verified for
  this record.
- Jupyter Server, *Security in the Jupyter Server*: one token per start grants whoever holds it the
  full rights of the server's account, with no further confirmation per action —
  <https://jupyter-server.readthedocs.io/en/latest/operators/security.html>
- Recalled, not verified for this record: that Jupyter Server and VS Code's server each keep one
  WebSocket per open client for their messages.
- `git-credential` and `gitcredentials`: helpers run as the user that runs git, from the
  configuration that user's git reads — <https://git-scm.com/docs/gitcredentials>
- `crates/commands/src/lib.rs`, `crates/server/src/socket.rs`, and `crates/server/src/turns.rs`:
  the shared `Host` and the list of commands each host answers, the WebSocket and what a closed
  tab gives up, and the turns, their bound, and the registry of open repositories.

## Consequences

- Positive: G-18's web interface runs against a repository on another machine with the desktop's
  answers, checks, and confirmations, from one implementation of the commands; a command added
  for one host reaches the other, and G-17's progress and stop reach the tab that started the
  transfer.
- Positive: the server builds without WebKit and runs on a machine without a display; Q-2's
  windows of rows cross the network as they cross Tauri's IPC, a screenful at a time.
- Positive (Q-5): a server started without the switch writes nothing and shares nothing between
  tabs but the process, so the default keeps what a stolen session costs to reading below the
  server root; with it, every write still needs the session cookie, the Origin check, and the
  root, and the bound keeps a session from tying up the blocking pool with waiting writes.
- Positive (Q-4): the server's writes are the git adapter's, which run no program the repository
  names; the server adds none.
- Positive (Q-7): a write that discards work runs only with the confirmation the shared `Host`
  checks, whatever tab or transport sends it; a second tab's write cannot land on a state the
  first changed, nor a closed tab's waiting write on a state nobody sees.
- Positive (Q-6): the browser never holds a credential; the repository's own helper does not run,
  as in the desktop.
- Negative / trade-offs: every open tab holds a laid-out history in the server's memory, so a
  developer with many tabs on a large repository pays for each, up to the bound on tabs; a
  dropped connection loses its tab's load session, and the tab loads its repository again.
- Negative / trade-offs (Q-5): with the switch, whoever holds a session writes and pushes with the
  developer's credentials, and a confirmation is a click the session can send; the switch, the
  cookie, and the root are the whole of what stands in the way.
- Negative / trade-offs (Q-6): a server started without an SSH agent in its environment, as a
  service usually is, cannot authenticate over SSH; the developer starts it where the agent runs,
  or uses HTTPS with a global helper.
- Negative / trade-offs (Q-5): a remote on the server's machine outside its root, which the
  desktop fetches from, is refused in the browser; the developer fetches it with `git`, or
  starts the server on a root that holds it.
- Negative / trade-offs: the commit a write makes shows the identity of the server account's
  configuration in the browser, where the working-directory node shows none.
- Negative / trade-offs: a write to a busy repository waits for the other tab's write, a push or
  a long fetch included, with nothing but its own pending answer to say so; a ninth waiting write
  is refused and must be sent again; closing a tab during a fetch loses what it had fetched so far.
- Follow-ups: whether a waiting write tells its tab that it waits; whether a write waits with a
  limit; whether the server prints at its start that it writes; whether the server bounds the
  memory its tabs hold.

## Enforcement

Tests that cite this ADR with a `Verifies:` marker, each against a server started in the test and
a repository made by `git` below its root where it writes:

- in `crates/commands/src/lib.rs`, against the shared `Host` with a recording event sink: a load
  reports each chunk, the refs after the first, and its end; a write waits for its turn after the
  root is checked, and the root is checked again once the turn comes; the shared list names each
  command once and each counterpart both ways;
- in `crates/server/src/lib.rs`: writes are allowed only by the switch given at the start;
- in `crates/server/src/web.rs`: the policy of ADR-0025's headers grants one origin beyond the
  server and no framing, and each inline script is hashed where a bundled one is not;
- in `crates/server/src/socket/scrub.rs`: a transfer's progress names no credential and no path on
  the server;
- in `crates/server/src/turns.rs`: a write waits for the turn of its repository and not for
  another's, and past the bound is refused at once, a closed tab's waiting write giving up
  unwritten;
- in `crates/server/src/turns/tabs.rs`: a write tells the other tabs on its repository and no tab
  that closed;
- in `crates/git/src/confine.rs`: an address is over the network only with a network scheme, or
  as `ssh` reads a host and a path;
- in `crates/git/tests/turn_reads.rs`: a worktree and its repository share one common directory
  and another repository does not, and the digest of the refs changes with a ref or `HEAD` and
  with nothing else;
- in `crates/git/tests/confined_remotes.rs`: confined to a folder, the git adapter fetches from
  and pushes to a repository on this machine only where it lies below the folder;
- in `crates/git/tests/git2_unwraps.rs`: each place of git2's source that panics by itself is
  audited, and a stash message, a configuration key, a remote's refusal of a push or its side
  band text, a submodule, or a working tree's link that a panicking call would read is read or
  refused without a panic;
- in `crates/git/tests/hostile_writes.rs`: every kind of write with each of its string fields set
  to each hostile value panics nowhere;
- in `crates/server/tests/server.rs`: the printed URL opens the repository at the root and the web
  interface's build is served with ADR-0025's headers, its rows arriving as the desktop host reads
  them; the server links no part of Tauri or WebKit, read from the lockfile's dependency tree;
  each read the browser asks is answered as the desktop host answers it; two tabs on two
  repositories each read their own, a selection staying in its tab; a tab closed or dropped
  during a load has its session dropped; a tab that does not read its answers is closed, and one
  that answers no ping is closed and frees its slot while one that reads stays; a repository
  swapped for a way out after it opened is checked again before a read or a write; the browser
  chooses a repository from the folders below the root, the counterpart of the desktop's folder
  dialog;
- in `crates/server/tests/command_list.rs`: each host answers every command the shared list gives
  it and no other, a command one host answers and the other or the list does not is named, the
  web interface sends no command the list does not name and listens for no event the hosts do
  not report, and a server event the server never reports or the shared commands report is
  named;
- in `crates/server/tests/allow_writes.rs`: without `--allow-writes` every write is refused and
  nothing changes, and with it the server answers every write the desktop answers, a write the
  desktop answers and the server answers otherwise or not at all named; a write git refuses
  changes nothing and names no path; a path that names no file of the working directory is
  refused and the server answers on; a repository whose hooks and configured programs are set
  runs none of them while the server stages, commits, stashes, merges, fetches, and pushes,
  where `git` performing the same writes runs each (Q-4); two tabs writing to one repository
  write in turn, the second refused with nothing changed where the first changed what it
  expected, and the other tab receives `repo-written` as its fixture in
  `crates/server/tests/fixtures/` says; a write past those waiting is refused, and a closed tab's
  waiting write never runs;
- in `crates/server/tests/writes.rs`: staging, unstaging, and committing leave the repository
  as the shared `Host` leaves it, and the load after the commit selects it; branches, stashes,
  tags, the history operations, and resolving, continuing, and aborting a stopped operation
  through the WebSocket each end as the desktop leaves them, and each write that discards work
  is refused without `confirmed` and performed with it (Q-7);
- in `crates/server/tests/transfers.rs`: fetching, pulling, and pushing end as the desktop leaves
  them and report `write-progress` to the tab that started them; a fetch and a pull's fetch stop
  on `cancel_write`; a tab closed while it fetches stops the fetch and gives the repository's turn
  back; a repository's own credential helper does not run (Q-6); a remote on this machine is
  fetched from and pushed to below the server root and refused outside it, by its path, by a
  `file://` address, and through `insteadOf` and `pushInsteadOf`;
- in `crates/server/tests/hostile_commands.rs`: every command of the shared list, sent at the
  WebSocket with each argument set to each hostile value, panics nowhere and is answered;
- in `src-web/src/transport.test.ts`: the WebSocket transport sends a command and resolves its
  answer by id, and delivers events, `repo-written` among them, to their listeners against the
  same fixtures the Tauri transport's types are held to;
- in `src-web/src/avatars.test.ts`: avatars are off until the developer turns them on, and every
  image the web interface draws from elsewhere stands inside that switch.
