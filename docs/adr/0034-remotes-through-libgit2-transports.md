# ADR-0034: Ocelli reaches remotes through libgit2's own transports with no proxy, credentials only from the developer's setup for a host both parsers read alike, no revoked SSH host key, and what a push took read without the remote's words

- **Status:** 🟢 accepted
- **Date:** 2026-10-10
- **Deciders:** Markus Brigl
- **Applies to:** fetching, pulling, and pushing in `crates/git/`, how a push learns what the remote took, the `git2` features in `crates/git/Cargo.toml`, how the front ends offer, stop, and reload after a transfer, and the tests of Q-4

## Context

G-12 ends with fetch, pull, and push, the only traffic Ocelli has with a remote repository. The
specification counts the developer's credentials (SSH keys, credential helpers) among the assets,
and a repository's configuration, the remote, and the network to it as untrusted. It asks that no
credential goes anywhere but to the remote it belongs to (Q-6 and the threats); Ocelli keeps none
of its own and uses the SSH agent and Git's credential helpers. Q-4 asks that no code the
repository brings runs, and a repository's configuration can name a credential helper, a proxy
command, or an SSH command. Q-5 holds the remote server to answering every request a browser may
make, which an abort does not, and to reading nothing outside the server root.
[ADR-0025](0025-no-egress-hardened-webview.md) keeps the WebView local and leaves Git's own network
operations out of its scope.

The `git` binary would bring every transport, proxy, and helper the developer has set up, and run
every program the repository's configuration names along with them. libgit2 brings HTTPS and SSH
of its own and runs no program unless the caller does. It reads no proxy unless the caller asks
it to. libgit2 offers no way to stop a push once its objects are sent. `git pull --ff-only` keeps
its fetch when it refuses a branch that has diverged.

Credential helpers need care. git2 tells a helper no host for an IP address, where a helper could
answer with any credential it holds. git2, through the `url` crate, and libgit2 read some
addresses differently. An address or user name with a line break is the class of CVE-2020-5260.
libssh2, through which libgit2 checks an SSH host key, reads only `~/.ssh/known_hosts`, not
`/etc/ssh/ssh_known_hosts`, and ignores the `@revoked` and `@cert-authority` markers.

The audit of git2's panicking places found that the callbacks a transfer uses unwrap strings from
outside as UTF-8. A release build aborts on a panic, every tab of the remote server with it:

- **`push_update_reference`**, the only place libgit2 hands over the reason a remote gives for
  each update, unwraps that reason. A remote that answers `ng refs/heads/main` with a reason that
  is not UTF-8 panics git2. The remote is a server on the network, so the bytes are not Ocelli's
  to vouch for; nothing Ocelli can check before the push sees them.
- **The certificate check** gets the host libgit2 connects to, which git2 unwraps as UTF-8.
  libgit2 hands every `scheme://` address but `http`, `https`, `git`, and `file` to SSH and
  percent-decodes its host, the part of the authority, which runs to the first `/`, after its
  last `@`; so `ssh://h%ff/r`, `ssh://h?%ff.x/r`, and `foo://h%ff.x:22/r` reach the unwrap once a
  server answers there. It decodes no address written as scp writes it, nor a user name into a
  host. An https server's redirect could name any host, and the check has nothing to add for
  https, where libgit2 verifies the certificate against the system's trust store; SSH has no
  redirect.
- **`update_tips`**, which libgit2 calls with the remote-tracking branch a fetch refspec of the
  remote maps a pushed branch to, unwraps that name as UTF-8; a refspec of the repository's
  configuration that is not UTF-8 reaches it.
- **git2's credential helper** splits a helper of `!` with nothing after it into no program and
  unwraps the first, where `sh` cannot start.

libgit2 calls `update_tips` after a push only for an update the remote reported taken, with no
reason, and only where a fetch refspec maps it to a remote-tracking branch, a negative refspec
not excluding it. Where none does (a clone of a single branch, a new branch outside the refspec,
a negative refspec), or where the server reports no status at all, it is not called, though
`git push` succeeds there. libgit2 tells, before it sends, where the remote's branch stands, in
its `push_negotiation` callback, which hands the ids over without unwrapping a name. A second
connection that reads the remote's branch again must reach the address the push went to:
libgit2 rewrites an address it is given with `url.<base>.insteadOf` once more, so the address it
pushed to, already rewritten, can lead a read elsewhere, outside the server root or from https
to SSH past the revoked-key check.

The maintainer decided on libgit2's transports, SSH through the running SSH agent, and HTTPS
through the credential helpers of the developer's own configuration, never one the repository's
configuration names. Pull only fast-forwards, and there is no force push for now. Ocelli refuses
no push `git push` makes. Only an update the remote is known not to have taken is called refused,
with what its hooks said on the side band. A branch found moved on by another push since is said
not to be confirmed. The remote's words are read in a form no panic follows from. The remote
server keeps answering a browser every failed write as `WRITE_FAILED`, so the remote's words reach
the desktop application and the terminal application only.

## Decision

We will fetch, pull, and push through libgit2's built-in HTTPS and SSH transports with no proxy,
authenticating with the running SSH agent, and with the credential helpers of the developer's
global and system configuration only for an HTTPS remote named by a domain that git2 and libgit2
read alike, verifying every server before any credential is sent, an SSH host key that any
`@revoked` line of the developer's or the system's `known_hosts` lists refused, and learning what
a push took from the remote-tracking branch libgit2 moves or, where it moves none, from the
remote's branch read again at the address pushed to, never from the reason the remote gives.

- **Transports.** `git2` with its `https` and `ssh` features. No transport runs a program: no
  `ext::`, no `core.sshCommand`, no `core.gitProxy`. No proxy: neither the repository's, the
  developer's, nor the environment's; a developer behind one fetches with `git`.
- **Credentials.** SSH keys only through the agent (`SSH_AUTH_SOCK`), the address's user or else
  the local one. A password only from `credential.helper` of the global and system
  configuration, never the repository's, and only over HTTPS to a remote named by an ASCII domain
  that both parsers read as the same host, after the last `@`; an IP address, an
  internationalised name, or an address with a line break, a backslash, a space, or a percent sign
  in its host is asked no helper, and the refusal says why. A helper git2 would run for the
  address, under the keys it reads, that is `!` with nothing after it is not run. Ocelli stores,
  logs, and shows no credential.
- **Servers.** TLS against the system's trust store, with no certificate check of Ocelli's own.
  An SSH host key must be in the developer's `known_hosts` as libssh2 reads it; before that,
  Ocelli refuses a key listed on any `@revoked` line of `~/.ssh/known_hosts` or
  `/etc/ssh/ssh_known_hosts`, whatever its host pattern. A host vouched for only by the system
  file or by `@cert-authority` is refused with a reason that says so. Nothing is trusted on first
  use. A server is given up after 30 s to connect or 60 s without a read.
- **Addresses.** The address a transfer checks is the one libgit2 connects to, rewritten by
  `url.<base>.insteadOf` and `pushInsteadOf` as libgit2 rewrites it. One that is not UTF-8 is
  refused before anything is sent, since the checks of the server root and of the credential
  helper read it as text. An address libgit2 hands to SSH as `scheme://` whose host, decoded as
  libgit2 decodes it, is not UTF-8 is refused.
- **Pull** fetches the checked-out branch's upstream and fast-forwards to it, checking the branch
  again under its lock after the fetch; a branch that has diverged is refused with the branch,
  the index, and the working directory as they were, the fetch kept as `git pull --ff-only`
  keeps it.
- **Push** sends the checked-out branch where `git push` would, after `push.default`,
  `branch.<name>.pushRemote`, and `remote.pushDefault`, refusing `nothing`, `matching`, an
  unknown mode, and a local upstream with a reason that says to use `git`; a branch without an
  upstream gets the one it was pushed to, `branch.<name>.merge` and `branch.<name>.remote` as
  `git push -u` sets them, the merge key first and taken out again should the remote key fail.
  No force push. A push libgit2 refuses before sending, as not a fast-forward of what the remote
  advertised, changes nothing and says the remote is ahead.
- **What a push took.** The push counts as taken where libgit2 moves the remote-tracking branch
  to the commit sent, which it does only for an update the remote reported taken; Ocelli listens
  for it only where every refspec of the remote is UTF-8. Where it does not move one, Ocelli
  reads the remote's branch again by fetching that one branch into a scratch reference, from an
  anonymous remote made of the first of three addresses that libgit2 rewrites to exactly the
  address pushed to, spelled as libgit2 spells a remote it makes (a UNC path with forward
  slashes on Windows): the remote's push address as configured, its address as configured, and
  the address pushed to itself; where none does, the push is said to be sent but not read. The
  scratch reference is named under `refs/ocelli/` for the process, the time it is made at, a
  count, and a random nonce, removed after the read, a failure to remove it said as such; one
  older than an hour, which only a process that ended mid-read leaves, is removed by the next
  read; no load and no digest of the refs counts a reference under `refs/ocelli/`. Where the
  remote's branch stood before the push is what libgit2's `push_negotiation` tells. The branch
  read holds:
  - the commit sent: the push is taken;
  - the commit it held before the push, or nothing where it had no such branch: the remote
    refused the update, said as "`<remote>` refused the update; the remote said: …" with the
    last lines it sent on its side band, its hooks' output among them, read lossily, ANSI escape
    sequences, control characters, and Unicode's default ignorable code points taken out, and
    bounded in length, or as "`<remote>` refused the update" where it sent none. A push the
    remote took and another push moved back to where the branch stood before reads as refused
    too: the two are the same branch to read;
  - another commit: another push may have moved it since, said as "`<remote>` did not confirm
    the update: its branch now points at another commit", with no commit named and no side band;
  - nothing where it had the branch before, or where libgit2 told nothing: said as
    "`<remote>` did not confirm the update: it has no such branch now".

  The message of an error libgit2 passes on from a remote, `remote error: …` and
  `report-status: Error reported: …` among them, is cleaned the same way before it is shown.
- **What reaches a browser.** The remote server answers a browser every failed write as
  `WRITE_FAILED`, as [ADR-0037](0037-git-adapter-confined-to-the-server-root.md) decides: the
  remote's side band reaches the desktop application and the terminal application only.
- **Stopping and after.** Off the thread that draws, with progress reported. A fetch, and a pull
  while it fetches, can be stopped; a push cannot, and no front end offers it a stop. A transfer
  done, or refused after it moved a ref, is followed by a reload.

**Out of scope:** force push; credentials typed into Ocelli; key files and passphrases; Git LFS;
submodule fetches; proxies; SSH certificates; showing the status line a remote gives for a
refused update.

## Alternatives considered

- **The `git` binary** — every transport and helper the developer has, but every program the
  repository's configuration names runs too (Q-4), and a write reaches the repository by a second
  way beside libgit2.
- **Any credential helper `git` would use** — the repository's helper included, as `git` does, but
  a repository could then name a program that receives the developer's credentials.
- **SSH key files with a passphrase prompt** — works without an agent, but Ocelli would hold a
  secret in memory and a prompt in its interface, an asset the specification keeps with the
  developer's own tools.
- **Trust on first use for SSH hosts** — less friction, but the first connection is the one an
  attacker on the network intercepts.
- **Pull by merge or rebase as configured** — what `git pull` does, but it brings G-13's merge and
  rebase, and their conflicts (G-14), into G-12.
- **A stop for a push, and a refused pull that undoes its fetch** — what a reader might expect,
  but libgit2 cannot stop a push once its objects are sent, and `git` keeps the fetch. Both go
  against the conventions that a write does what `git` does and that a control offers no stop it
  cannot keep.
- **Proxies from the developer's global configuration and the environment, never the
  repository's** — works behind a corporate proxy; brings proxy credentials, a second network
  hop to reason about, and parsing of `http.proxy` and `NO_PROXY`. The candidate if a developer
  needs it; not taken now, as the simplest form that stays honest.
- **libgit2's automatic proxy detection** — reads the repository's `http.proxy` too: a cloned
  repository could route the developer's traffic through a host it names.
- **Ocelli's own `known_hosts` check in full,** hashed names, patterns, ports, and
  `@cert-authority` — what `ssh` does, but a security-critical parser of Ocelli's own with new
  dependencies; refusing revoked keys needs only the key, compared byte for byte.
- **Refuse every SSH connection while any `known_hosts` file holds a marker line** — fails closed
  with no parsing at all, but locks out every developer whose `known_hosts` uses a certificate
  authority for one host.
- **Speak the credential protocol to the helper ourselves,** with the host libgit2 connects to —
  serves IP and internationalised hosts, but Ocelli would build helper input itself, the very
  place CVE-2020-5260 lived; SSH serves those hosts today.
- **Read each update's status from git2's `push_update_reference`** — the only place libgit2
  hands over the reason a remote gives for each update, but git2 unwraps it, and a remote then
  aborts the remote server.
- **A patched git2, by `[patch.crates-io]` or a fork, reading the reason lossily** — keeps the
  reason, but makes Ocelli carry a fork of its git binding and re-apply the patch on every
  upgrade, a supply-chain decision of its own; the side band carries what a hook says.
- **Refuse a push `update_tips` cannot confirm** — no second connection, but refuses pushes
  `git push` makes: a single-branch clone, a new branch outside the fetch refspec, a negative
  refspec, a server without a status report. The maintainer rejected it.
- **Read the remote's references with `Remote::list`** — writes no reference, but git2 unwraps
  each name the remote sends as UTF-8 in `RemoteHead::name`, the very panic avoided, and has no
  form of it in bytes.
- **Read the branch into `FETCH_HEAD`** — writes no reference, but overwrites the developer's
  `FETCH_HEAD`, and git2 unwraps its names as UTF-8 in `fetchhead_foreach`.
- **One fixed scratch reference** — simpler, but two pushes or two Ocelli processes at once read
  each other's, and one a developer made under the same name is overwritten.
- **Call every push not confirmed refused** — one message, but a push another one overtook would
  read as refused, with a hook's text that may belong to neither.
- **Count a push as taken when libgit2 returns no error** — libgit2 returns success for an update
  the remote refused; a refused push would read as done.
- **Unsafe code against libgit2-sys** — the workspace denies unsafe code, and the callbacks git2
  wraps are where the panics live.

## Sources / Prior art

- libgit2's transports, credential callbacks, and certificate checks — <https://libgit2.org/>;
  `git2::RemoteCallbacks`, `git2::ProxyOptions`, `git2::opts` — <https://docs.rs/git2>.
- libgit2 1.9.7 as locked through `libgit2-sys` 0.18.8: `git_push_update_tips` and `do_push` in
  `push.c` (no update for a status with a reason, none where no fetch refspec matches; the
  negotiation before sending); `git_remote_upload`, `git_remote_create_with_opts`, and
  `apply_insteadof` in `remote.c` (the status callback only where registered; an address
  rewritten again on every remote made); `parse_report` in `transports/smart_protocol.c`
  (side-band progress during a push's report); `transport_find_fn` in `transport.c` (which
  addresses go to SSH); `git_net_url_parse`, `url_parse_authority`, `url_parse_finalize`, and
  `git_net_url_parse_scp` in `util/net.c` (which hosts are percent-decoded).
- git2 0.20.4: `push_update_reference_cb`, `update_tips_cb`, `certificate_check_cb`,
  `sideband_progress_cb`, and `push_negotiation_cb` in `remote_callbacks.rs`; `CredentialHelper`
  in `cred.rs`; the audit of its panicking places in `crates/git/tests/git2_unwraps.txt`.
- `gitcredentials` and the credential protocol — <https://git-scm.com/docs/gitcredentials>,
  <https://git-scm.com/docs/git-credential>; CVE-2020-5260 —
  <https://github.com/git/git/security/advisories/GHSA-qm7j-c969-7j4q>.
- `git-pull --ff-only` — <https://git-scm.com/docs/git-pull>.
- `git-push`, `push.default`, `url.<base>.insteadOf`, and the `report-status` capability —
  <https://git-scm.com/docs/git-push>, <https://git-scm.com/docs/git-config>,
  <https://git-scm.com/docs/protocol-capabilities>.
- `sshd(8)`, SSH_KNOWN_HOSTS FILE FORMAT, for `@revoked` and `@cert-authority` —
  <https://man.openbsd.org/sshd.8#SSH_KNOWN_HOSTS_FILE_FORMAT>; libssh2's `known_hosts` support —
  <https://libssh2.org/libssh2_knownhost_readfile.html>.
- The specification's threat model, which names the credentials as assets.

## Consequences

- Positive: no program the repository names runs on the way to a remote (Q-4); credentials go
  only to the remote they belong to, from the developer's own setup; no server is trusted unseen.
- Positive: no reply of a remote reaches a panic of git2 in a transfer, so a remote cannot abort
  the remote server (Q-5); a push `git push` makes is not refused; a hook's refusal tells the
  developer at the desktop or the terminal what the hook said; a push overtaken is not called
  refused; the read-back reads nothing from an address the push was not checked for; a revoked
  SSH key is refused; no proxy a repository names, and none at all, sees the traffic (Q-4, Q-6);
  a reader takes neither a push without a stop nor a kept fetch for a mistake.
- Negative / trade-offs: native TLS and SSH libraries join the build and its supply chain; a
  developer whose SSH key is not in an agent, or whose host is not in `known_hosts`, has to set
  that up first; a credential helper set per repository does not work; a branch that has diverged
  is fetched and then merged or rebased from the commit graph (G-13), or pulled with `git`.
- Negative / trade-offs: a push that `update_tips` does not confirm opens a second connection to
  the remote, asking for credentials again, and writes and removes a scratch reference under
  `refs/ocelli/`; one whose configured addresses `insteadOf` would all lead elsewhere is reported
  as sent but not read; the status line of a refused update is not shown, only the side band, and
  a browser sees neither; an address libgit2 hands to SSH whose host decodes to bytes that are not
  UTF-8, a remote whose address is not UTF-8, and a helper of `!` alone are refused where `git`
  would try them. A developer behind a proxy, one whose hosts are vouched for by a certificate
  authority or only in the system's `known_hosts`, and one whose HTTPS remote is named by an IP
  address or an internationalised name must use `git` or SSH; a revoked key is refused for every
  host, a stricter rule than `ssh`'s.
- Follow-ups: force push with a lease and a confirmation; merging and rebasing on pull, as G-13
  merges and rebases; proxies from the developer's own setup; SSH certificates; the reason of a
  refused update, should git2 hand it over in bytes.

## Enforcement

The tests of the git adapter cite this ADR with `Verifies:`, against bare remotes made with `git`
over the local file transport and local servers: fetch, pull, and push as `git` does, the pull's
refusal keeping the fetch, `push.default` compared with `git push`, a commit made during a pull's
fetch kept; in `crates/git/tests/git2_unwraps.rs`, a server of the git protocol on a local port that
refuses a push with a reason that is not UTF-8, for a branch it holds and one it has not, with a
hook's text on the side band, and with side-band text that is not UTF-8 and holds escape sequences,
control and invisible characters, none of which panics; that takes a push no fetch refspec maps and
one a negative refspec leaves untracked, both counted as taken from the remote's branch read again;
that holds the branch at another commit, which is not confirmed and not called refused; a read-back
under an `insteadOf` chain that would lead outside, which reads only the address pushed to; and that
no scratch reference is left; the credential tests in `crates/git/tests/write_credentials.rs` and
the unit tests of the address check, the line-break guard, the helper of `!` alone, and the decoding
of a host libgit2 hands to SSH; a unit test of the `@revoked` check over a key and file contents; a
structural test that no source of the git adapter gives a transfer a proxy; the front ends' tests
that a push offers no stop and that a refused pull reloads; the remote server's tests that a failed
write reaches a browser as `WRITE_FAILED`; the Q-4 test for writing, performing every write. A
structural test reads the `git2` features. The audit of git2's panicking places fails on any upgrade
that changes one. **Not mechanically decidable:** HTTPS and SSH against real servers, which the
maintainer checks.
