//! The transfers of the repository port, with git2: fetching, pulling, and pushing through
//! libgit2's own transports, the credentials a remote is given, the host keys it is checked
//! against, and the server timeouts (ADR-0035). No transfer runs a program the repository names
//! (Q-4).

use crate::checkout::check_out;
use crate::guard::{branch_as_shown, refname_of};
use anyhow::{Context, Result, bail};
use git2::{BranchType, ErrorCode, Oid, Repository};

/// Fetches `remote` through libgit2's own transports, as `git fetch <remote>` does, telling
/// `progress` how far it has come and stopping where it says so (ADR-0035).
pub(crate) fn fetch(
    repo: &Repository,
    name: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<()> {
    let mut remote = repo
        .find_remote(name)
        .with_context(|| format!("there is no remote named '{name}' any more"))?;
    let url = address(remote.url_bytes(), name)?;
    crate::confine::remote_within_root(repo.path(), name, &url)?;
    let transfer = Transfer::new("Fetching", name, &url, progress)?;
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(transfer.callbacks());
    let fetched = remote.fetch(&[] as &[&str], Some(&mut options), None);
    drop(options);
    fetched.map_err(|e| transfer.refusal(e, &url, "fetching from"))
}

/// One transfer with a remote, a fetch or a push, through libgit2's own transports (ADR-0035):
/// what it tells `progress`, whether the developer stopped it, and how a failure reads. No
/// credential helper or command the repository's configuration names runs, and no server is
/// trusted unseen: libgit2 verifies a certificate against the system's trust store and an SSH
/// host key against the developer's `known_hosts`, since no certificate check of Ocelli's own
/// overrides it. A server that stalls is given up after [`SERVER_TIMEOUT_MS`].
struct Transfer<'p> {
    doing: &'static str,
    name: String,
    progress: std::cell::RefCell<&'p mut dyn FnMut(&str) -> bool>,
    cancelled: std::cell::Cell<bool>,
    tried: std::cell::RefCell<Tried>,
    /// The `known_hosts` that revoked the server's key, where one did: libssh2 replaces the
    /// message of a refusal with its own, so the reason is kept here.
    revoked: std::cell::RefCell<Option<String>>,
    /// Whether the server may be an SSH one, whose host key is checked.
    checks_host_key: bool,
    /// The last of what the server sent on its side band, hook output among it, as it came.
    told: std::cell::RefCell<Vec<u8>>,
}

/// The most of the side band a transfer keeps, the end of it.
const TOLD_KEPT: usize = 4096;

impl<'p> Transfer<'p> {
    /// A transfer with the remote `name` at `url`. git2 unwraps the host it hands the
    /// certificate check as UTF-8, and libgit2 percent-decodes the host of an `ssh://` address,
    /// though not of one written as scp writes it: an `ssh://` address whose host decodes to
    /// bytes that are not UTF-8 is refused. An http or https server, whose redirect could name
    /// any host, is not given the check, which has nothing to add there; SSH has no redirect.
    fn new(
        doing: &'static str,
        name: &str,
        url: &str,
        progress: &'p mut dyn FnMut(&str) -> bool,
    ) -> Result<Self> {
        let lower = url.to_ascii_lowercase();
        let checks_host_key = !lower.starts_with("https://") && !lower.starts_with("http://");
        if checks_host_key && !ssh_host_decodes_to_utf8(url) {
            bail!(
                "the host in the address of {name} decodes to bytes that are not UTF-8, which \
                 Ocelli does not take: use git"
            );
        }
        set_server_timeouts();
        Ok(Self {
            doing,
            name: name.to_string(),
            progress: std::cell::RefCell::new(progress),
            cancelled: std::cell::Cell::new(false),
            tried: Default::default(),
            revoked: Default::default(),
            checks_host_key,
            told: Default::default(),
        })
    }

    /// The last lines the server sent on its side band, read lossily, escape sequences, control
    /// characters, and invisible formatting characters taken out, at most a few and a few
    /// hundred characters; none where it sent nothing.
    fn told(&self) -> Option<String> {
        let told = self.told.borrow();
        let kept = cleaned(&String::from_utf8_lossy(&told));
        (!kept.is_empty()).then_some(kept)
    }

    /// Tells the developer how far the transfer has come; false once they stopped it.
    fn go_on(&self, how_far: &str) -> bool {
        let go_on =
            (self.progress.borrow_mut())(&format!("{} {}: {how_far}", self.doing, self.name));
        self.cancelled.set(!go_on);
        go_on
    }

    fn callbacks(&self) -> git2::RemoteCallbacks<'_> {
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|url, user, allowed| {
            if !self.go_on("authenticating") {
                return Err(git2::Error::from_str("cancelled"));
            }
            self.tried.borrow_mut().credentials(url, user, allowed)
        });
        // libgit2 checks an SSH host key against the developer's known_hosts after this; a key
        // that a revoked line lists, which libssh2 does not read, is refused first (ADR-0035).
        if self.checks_host_key {
            self.check_host_key(&mut callbacks);
        }
        // git2 hands the side band over as bytes, unread.
        callbacks.sideband_progress(|text| {
            let mut told = self.told.borrow_mut();
            told.extend_from_slice(text);
            let over = told.len().saturating_sub(TOLD_KEPT);
            told.drain(..over);
            drop(told);
            self.go_on("the server is preparing")
        });
        callbacks.transfer_progress(|stats| {
            self.go_on(&format!(
                "{} of {} objects",
                stats.received_objects(),
                stats.total_objects()
            ))
        });
        // libgit2 offers no way to stop a push once its objects are sent: told, not asked.
        callbacks.push_transfer_progress(|current, total, _| {
            (self.progress.borrow_mut())(&format!(
                "{} {}: {current} of {total} objects",
                self.doing, self.name
            ));
        });
        callbacks
    }

    /// Refuses an SSH host key that a revoked line of the developer's `known_hosts` lists.
    #[expect(
        clippy::disallowed_methods,
        reason = "only for an address whose host libgit2 decodes to UTF-8, and never for an http \
                  server, whose redirect could name any host"
    )]
    fn check_host_key<'s>(&'s self, callbacks: &mut git2::RemoteCallbacks<'s>) {
        callbacks.certificate_check(|cert, host| {
            if let Some(key) = cert.as_hostkey().and_then(|k| k.hostkey()) {
                let files = known_hosts_files();
                if let Some(file) = files.iter().find(|(_, text)| revoked_in(text, key)) {
                    *self.revoked.borrow_mut() = Some(file.0.clone());
                    return Err(git2::Error::new(
                        ErrorCode::Certificate,
                        git2::ErrorClass::Ssh,
                        format!("{host}'s host key is revoked in {}", file.0),
                    ));
                }
            }
            Ok(git2::CertificateCheckStatus::CertificatePassthrough)
        });
    }

    /// What a failed transfer tells the developer.
    fn refusal(&self, e: git2::Error, url: &str, direction: &str) -> anyhow::Error {
        let name = &self.name;
        let doing = self.doing.to_lowercase();
        if self.cancelled.get() {
            return anyhow::anyhow!("{doing} {name} was cancelled");
        }
        match e.code() {
            ErrorCode::Certificate if self.revoked.borrow().is_some() => anyhow::anyhow!(
                "{name} cannot be trusted: its host key is revoked in {}",
                self.revoked.borrow().as_deref().unwrap_or_default()
            ),
            ErrorCode::Certificate if is_ssh(url) => {
                let files = known_hosts_files();
                let user = files.first().map_or("", |f| f.1.as_str());
                let system = files.get(1).map_or("", |f| f.1.as_str());
                let host = ssh_host(url);
                match vouched_elsewhere(user, system, host) {
                    Some(why) => anyhow::anyhow!(
                        "{name}'s host key is {why}, which Ocelli does not trust: add the host to \
                         your ~/.ssh/known_hosts"
                    ),
                    None => anyhow::anyhow!(
                        "{name}'s host key is not in your known_hosts: connect to it once with \
                         ssh, and check its fingerprint, before {direction} it from Ocelli"
                    ),
                }
            }
            _ if e.code() == ErrorCode::Certificate || e.class() == git2::ErrorClass::Ssl => {
                anyhow::anyhow!(
                    "{name}'s server could not be verified against the system's certificates: {}",
                    cleaned(e.message())
                )
            }
            ErrorCode::Auth => {
                anyhow::anyhow!("{name} refused the credentials: {}", cleaned(e.message()))
            }
            ErrorCode::NotFastForward => {
                anyhow::anyhow!(
                    "{name} has commits the branch does not: the remote is ahead, so pull first"
                )
            }
            // libgit2 passes on what a remote sends as an error, `remote error: …` or
            // `report-status: Error reported: …`, in its message.
            _ => anyhow::anyhow!("{}", cleaned(e.message())),
        }
    }
}

/// Pulls the checked-out branch `name`, still at `tip`, as `git pull --ff-only` does: fetches
/// its upstream's remote and fast-forwards to the upstream, the uncommitted changes carried along;
/// a branch that has diverged is refused, the fetch kept (ADR-0035).
pub(crate) fn pull(
    repo: &Repository,
    name: &str,
    tip: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<()> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: a pull moves the checked-out branch");
    }
    let refname = refname_of(&branch)?;
    let remote = repo
        .branch_upstream_remote(&refname)
        .ok()
        .and_then(|remote| remote.as_str().map(str::to_string))
        .with_context(|| format!("'{name}' has no upstream to pull from"))?;
    // An upstream on `.` is a local branch, with nothing to fetch.
    if remote != "." {
        fetch(repo, &remote, progress)?;
    }
    let upstream = repo
        .find_branch(name, BranchType::Local)?
        .upstream()
        .map_err(|_| anyhow::anyhow!("the upstream of '{name}' is not there to pull from"))?;
    let theirs = upstream
        .get()
        .target()
        .context("the upstream points at nothing")?;
    let ours = Oid::from_str(tip)?;
    if theirs == ours || repo.graph_descendant_of(ours, theirs)? {
        return Ok(());
    }
    if !repo.graph_descendant_of(theirs, ours)? {
        bail!(
            "'{name}' and its upstream have diverged: merge or rebase them with git; only the \
             fetch was done"
        );
    }
    let target = repo.find_commit(theirs)?;
    // The branch is locked before anything changes, and checked against `tip` again under the
    // lock: a commit made while the fetch ran is neither lost nor checked out over, and a lock
    // another program holds refuses the pull before the working directory moves.
    let mut transaction = repo.transaction()?;
    crate::guard::lock_ref(&mut transaction, &refname)
        .with_context(|| format!("'{name}' is locked by another program: try again later"))?;
    branch_as_shown(repo, name, tip)?;
    check_out(repo, &target)?;
    crate::guard::set_target(&mut transaction, &refname, theirs, "pull: Fast-forward")?;
    transaction.commit()?;
    Ok(())
}

/// Pushes the checked-out branch `name`, still at `tip`, as `git push` does, never by force: to
/// its upstream, or to the default remote under its own name, which then becomes its upstream. A push the remote refuses, one that is not a fast-forward, changes
/// nothing (ADR-0035).
pub(crate) fn push(
    repo: &Repository,
    name: &str,
    tip: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<()> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: a push sends the checked-out branch");
    }
    let refname = refname_of(&branch)?;
    let (remote_name, destination, first) = push_destination(repo, name, &refname)?;
    let mut remote = repo
        .find_remote(&remote_name)
        .with_context(|| format!("there is no remote named '{remote_name}' any more"))?;
    let url = address(
        remote.pushurl_bytes().unwrap_or(remote.url_bytes()),
        &remote_name,
    )?;
    crate::confine::remote_within_root(repo.path(), &remote_name, &url)?;
    // libgit2 hands the reason a remote gives for refusing an update to the
    // push_update_reference callback, where git2 unwraps it as UTF-8, and a release build aborts
    // on the panic. What the remote took is told instead by update_tips, which libgit2 calls only
    // for an update the remote reported taken, where a fetch refspec maps it to a
    // remote-tracking branch; where it is not called, the remote's branch is read again.
    let sent = Oid::from_str(tip)?;
    // git2 unwraps the name update_tips is given as UTF-8: one a fetch refspec of the remote maps
    // the destination, itself UTF-8, to, which is UTF-8 where every refspec is.
    let names_utf8 = remote.refspecs().all(|refspec| refspec.str().is_some());
    let transfer = Transfer::new("Pushing", &remote_name, &url, progress)?;
    let taken = std::cell::Cell::new(false);
    // Where the remote's branch stood before the push, as libgit2 negotiated it: the one update
    // sent, its source the remote's commit, zero for a branch the remote does not have.
    let before = std::cell::Cell::new(None);
    let mut callbacks = transfer.callbacks();
    callbacks.push_negotiation(|updates| {
        if let [update] = updates {
            before.set(Some(update.src()));
        }
        Ok(())
    });
    if names_utf8 {
        #[expect(
            clippy::disallowed_methods,
            reason = "registered only where every refspec of the remote, and so every name \
                      libgit2 maps the destination to, is UTF-8"
        )]
        callbacks.update_tips(|_, _, to| {
            // libgit2 calls it only for the one update sent, and only where the remote took it.
            taken.set(taken.get() || to == sent);
            true
        });
    }
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    // No `+`: a push that is not a fast-forward is the remote's to refuse.
    let pushed = remote.push(&[format!("{refname}:{destination}")], Some(&mut options));
    drop(options);
    pushed.map_err(|e| transfer.refusal(e, &url, "pushing to"))?;
    let told = transfer.told();
    drop(transfer);
    if !taken.get() {
        let now = read_back(repo, &remote_name, &url, &destination, progress)?;
        // A branch where it stood before was not moved: the remote refused the update. One that
        // stands elsewhere may have been moved by another push since, which this one cannot
        // tell from a refusal.
        let before = before.get().map(|oid| (!oid.is_zero()).then_some(oid));
        match now {
            Some(now) if now == sent => {}
            now if Some(now) == before => match told {
                Some(told) => {
                    bail!("{remote_name} refused the update; the remote said: {told}")
                }
                None => bail!("{remote_name} refused the update"),
            },
            Some(_) => bail!(
                "{remote_name} did not confirm the update: its branch now points at another commit"
            ),
            None => bail!("{remote_name} did not confirm the update: it has no such branch now"),
        }
    }
    if first {
        set_upstream(repo, name, &remote_name, &destination)
            .with_context(|| format!("'{name}' was pushed, but its upstream could not be set"))?;
    }
    Ok(())
}

/// Sets the upstream of the branch `name` to `destination` on `remote`, as `git push -u` sets it,
/// whether or not a remote-tracking branch follows it. git2 locks no configuration across two
/// keys: the merge key is written first, which names no upstream on its own, and put back as it
/// was, or taken out where there was none, should the remote not follow, so that a failure
/// leaves no half an upstream and keeps a merge key the developer had.
fn set_upstream(repo: &Repository, name: &str, remote: &str, destination: &str) -> Result<()> {
    let mut config = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    let merge = format!("branch.{name}.merge");
    let had = config.snapshot()?.get_string(&merge).ok();
    config.set_str(&merge, destination)?;
    if let Err(e) = config.set_str(&format!("branch.{name}.remote"), remote) {
        let _ = match had {
            Some(had) => config.set_str(&merge, &had),
            None => config.remove(&merge),
        };
        return Err(e.into());
    }
    Ok(())
}

/// Where Ocelli keeps the references it makes for itself, which no load shows.
pub(crate) const OWN_REFS: &str = "refs/ocelli/";

/// How long a scratch reference of a read-back is kept before another read-back takes it for
/// one a process that ended left behind: far longer than the server timeouts let one last.
const SCRATCH_KEPT_SECS: u64 = 3600;

/// `url` as libgit2 spells the address of a remote it makes from it: on Windows a UNC path,
/// `\\server\share\…`, with forward slashes, as core git writes it; any other as it stands.
fn as_libgit2_makes_it(url: &str, windows: bool) -> String {
    let unc = url.starts_with("\\\\")
        && url[2..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if windows && unc {
        url.replace('\\', "/")
    } else {
        url.to_string()
    }
}

/// A scratch reference for one read-back, named for this process, the time it is made at, a
/// count, and a random nonce, so that neither another read-back, another Ocelli process, nor a
/// name anyone could guess ahead uses it.
fn scratch_ref() -> (String, u64) {
    use std::hash::{BuildHasher, Hasher};
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // The standard library seeds each `RandomState` from the system's randomness.
    let nonce = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let name = format!(
        "{OWN_REFS}pushed-{}-{now}-{count}-{nonce:016x}",
        std::process::id()
    );
    (name, now)
}

/// Removes the scratch references of read-backs older than [`SCRATCH_KEPT_SECS`], which a
/// process that ended mid-way left behind; only names of the scratch form are touched.
fn remove_stale_scratch(repo: &Repository, now: u64) {
    let Ok(references) = repo.references_glob(&format!("{OWN_REFS}pushed-*")) else {
        return;
    };
    for mut reference in references.flatten() {
        let made = reference
            .name()
            .and_then(|n| n.strip_prefix(&format!("{OWN_REFS}pushed-")))
            .and_then(|rest| rest.split('-').nth(1))
            .and_then(|secs| secs.parse::<u64>().ok());
        if made.is_some_and(|made| now.saturating_sub(made) > SCRATCH_KEPT_SECS) {
            let _ = reference.delete();
        }
    }
}

/// The commit the remote named `name`, pushed to at `url`, holds at `destination` now, none
/// where it has no such branch, read by fetching that one branch into a scratch reference of
/// this read-back's own, which is gone again after.
///
/// The fetch goes to `url` and nowhere else: libgit2 rewrites an address it is given with
/// `url.<base>.insteadOf` once more, so the read-back takes the first of the remote's push
/// address as configured, its address as configured, and `url` itself whose rewriting is `url`,
/// spelled as libgit2 spells a remote it makes ([`as_libgit2_makes_it`]), and is refused where
/// none is.
fn read_back(
    repo: &Repository,
    name: &str,
    url: &str,
    destination: &str,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<Oid>> {
    let config = repo.config()?.snapshot()?;
    let configured = [
        config.get_string(&format!("remote.{name}.pushurl")).ok(),
        config.get_string(&format!("remote.{name}.url")).ok(),
        Some(url.to_string()),
    ];
    let mut remote = configured
        .into_iter()
        .flatten()
        .filter_map(|address| repo.remote_anonymous(&address).ok())
        .find(|remote| remote.url_bytes() == as_libgit2_makes_it(url, cfg!(windows)).as_bytes())
        .with_context(|| {
            format!(
                "the push to {name} was sent, but whether it took it cannot be read: \
                 url.<base>.insteadOf would lead the read elsewhere"
            )
        })?;
    let (scratch, now) = scratch_ref();
    remove_stale_scratch(repo, now);
    let transfer = Transfer::new("Checking", name, url, progress)?;
    let mut options = git2::FetchOptions::new();
    options
        .remote_callbacks(transfer.callbacks())
        .download_tags(git2::AutotagOption::None)
        .update_fetchhead(false);
    let fetched = remote.fetch(
        &[format!("+{destination}:{scratch}")],
        Some(&mut options),
        None,
    );
    drop(options);
    let read = repo.find_reference(&scratch).ok().and_then(|r| r.target());
    if let Ok(mut reference) = repo.find_reference(&scratch) {
        reference.delete().with_context(|| {
            format!(
                "the push to {name} was sent and read back, but the reference {scratch} it was \
                 read into could not be removed: delete it with git"
            )
        })?;
    }
    fetched
        .map_err(|e| transfer.refusal(e, url, "checking"))
        .with_context(|| {
            format!("the push to {name} was sent, but whether it took it could not be read")
        })?;
    Ok(read)
}

/// Where a push of the branch `name` goes, as `git push` decides with `push.default` and the
/// branch's push remote: the remote, the reference there, and whether the branch has no upstream
/// yet, which the push then sets, as `git push -u` would. `simple`, the default, refuses an
/// upstream named differently; `nothing` and `matching` are left to `git`.
fn push_destination(
    repo: &Repository,
    name: &str,
    refname: &str,
) -> Result<(String, String, bool)> {
    let config = repo.config()?;
    let setting = |key: &str| config.get_string(key).ok().filter(|v| !v.is_empty());
    let upstream = repo
        .branch_upstream_remote(refname)
        .ok()
        .and_then(|remote| remote.as_str().map(str::to_string));
    if upstream.as_deref() == Some(".") {
        bail!("the upstream of '{name}' is a local branch, which a push does not reach");
    }
    let remote = setting(&format!("branch.{name}.pushRemote"))
        .or_else(|| setting("remote.pushDefault"))
        .or_else(|| upstream.clone())
        .or_else(|| crate::refs::default_remote(repo, &crate::refs::remote_names(repo)))
        .with_context(|| format!("there is no remote to push '{name}' to"))?;
    let merge = setting(&format!("branch.{name}.merge"));
    let to_upstream = upstream.as_deref() == Some(remote.as_str());
    let mode = setting("push.default").unwrap_or_else(|| "simple".to_string());
    let destination = match mode.as_str() {
        "nothing" => bail!("push.default is nothing: name what to push with git"),
        "matching" => bail!("push.default is matching, which pushes every branch: use git for it"),
        "current" => refname.to_string(),
        "upstream" | "tracking" => match merge.filter(|_| to_upstream) {
            Some(merge) if merge.starts_with("refs/heads/") => merge,
            _ => bail!("'{name}' has no upstream branch on {remote} to push to"),
        },
        "simple" => match merge.filter(|_| to_upstream) {
            Some(merge) if merge != refname => bail!(
                "the upstream of '{name}' is named differently ({merge}): push it with git, \
                 naming where"
            ),
            _ => refname.to_string(),
        },
        other => bail!("push.default is {other}, which Ocelli does not know: use git"),
    };
    Ok((remote, destination, upstream.is_none()))
}

/// How long libgit2 waits for a server to connect, and then for each read, before it gives up.
const SERVER_CONNECT_TIMEOUT_MS: i32 = 30_000;
const SERVER_TIMEOUT_MS: i32 = 60_000;

/// Sets libgit2's server timeouts, once for the process, before the first transfer: without them
/// a server that stalls holds a fetch, and the developer's next write, for ever.
#[allow(unsafe_code)]
fn set_server_timeouts() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: libgit2's global options are set here once, before any transfer reads them;
        // the reads of a repository do not use them.
        unsafe {
            let _ =
                git2::opts::set_server_connect_timeout_in_milliseconds(SERVER_CONNECT_TIMEOUT_MS);
            let _ = git2::opts::set_server_timeout_in_milliseconds(SERVER_TIMEOUT_MS);
        }
    });
}

/// The developer's and the system's `known_hosts`, by path and text, the developer's first; a file
/// that cannot be read counts as empty. The home is found as libgit2 finds it: `HOME`, else on
/// Windows `USERPROFILE` or `HOMEDRIVE` and `HOMEPATH`; the system's file is where OpenSSH keeps it.
fn known_hosts_files() -> Vec<(String, String)> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let home = var("HOME")
        .or_else(|| var("USERPROFILE"))
        .or_else(|| Some(format!("{}{}", var("HOMEDRIVE")?, var("HOMEPATH")?)));
    let system = if cfg!(windows) {
        var("ProgramData").map(|data| format!("{data}\\ssh\\ssh_known_hosts"))
    } else {
        Some("/etc/ssh/ssh_known_hosts".to_string())
    };
    let user = home.map(|home| {
        std::path::Path::new(&home)
            .join(".ssh")
            .join("known_hosts")
            .display()
            .to_string()
    });
    [user, system]
        .into_iter()
        .map(|path| {
            let text = path
                .as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            (path.unwrap_or_default(), text)
        })
        .collect()
}

/// Whether a `@revoked` line of `known_hosts` lists `key`, whatever its host pattern.
fn revoked_in(known_hosts: &str, key: &[u8]) -> bool {
    known_hosts.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some("@revoked")
            && fields
                .nth(2)
                .is_some_and(|encoded| decode_base64(encoded).as_deref() == Some(key))
    })
}

/// Where a host libssh2 refused is vouched for all the same: only in the system's file, or only
/// through `@cert-authority`, neither of which libssh2 reads; `None` where it is not.
fn vouched_elsewhere(user: &str, system: &str, host: &str) -> Option<&'static str> {
    let names = |line: &str, field: usize| {
        line.split_whitespace().nth(field).is_some_and(|patterns| {
            patterns
                .split(',')
                .any(|p| p == host || p.contains(&format!("[{host}]")))
        })
    };
    if user
        .lines()
        .chain(system.lines())
        .any(|l| l.starts_with("@cert-authority") && names(l, 1))
    {
        return Some("vouched for only by a @cert-authority line");
    }
    if system
        .lines()
        .any(|l| !l.starts_with('@') && !l.starts_with('#') && names(l, 0))
    {
        return Some("listed only in /etc/ssh/ssh_known_hosts");
    }
    None
}

/// The host an SSH address names: `<scheme>://[user@]host[:port]/path`, any scheme [`ssh_url`]
/// reads, or `[user@]host:path`.
fn ssh_host(url: &str) -> &str {
    let rest = ssh_url(url).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    host.split(':').next().unwrap_or_default()
}

/// Standard base64, as `known_hosts` writes keys; `None` for text that is not.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut held = 0;
    let mut out = Vec::new();
    for c in text.trim_end_matches('=').bytes() {
        let value = TABLE.iter().position(|&t| t == c)? as u32;
        bits = bits << 6 | value;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    Some(out)
}

/// `text` without its ANSI escape sequences: a control sequence (`ESC [` up to its final byte),
/// an operating system command (`ESC ]` up to BEL or `ESC \`), and any other escape with the one
/// character after it.
fn without_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || c == '\u{1b}' && chars.next_if_eq(&'\\').is_some() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `c` is shown as written: no control character, none of Unicode's default ignorable
/// code points, which render as nothing and can reorder or hide text (the bidirectional
/// controls, the zero-width characters, the variation selectors, the tag characters, the Hangul
/// fillers among them), and no interlinear annotation or line and paragraph separator.
fn shown(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{ad}'
                | '\u{34f}'
                | '\u{61c}'
                | '\u{115f}'..='\u{1160}'
                | '\u{17b4}'..='\u{17b5}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff0}'..='\u{fffb}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0000}'..='\u{e0fff}'
        )
}

/// Text a remote sent, as Ocelli shows it: read lossily before, ANSI escape sequences, control
/// characters, and characters not [`shown`] taken out, its last five lines that hold anything
/// joined by ` / `, and at most 400 characters of them.
fn cleaned(text: &str) -> String {
    let lines: Vec<String> = without_escapes(text)
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .map(|line| line.chars().filter(|&c| shown(c)).collect::<String>())
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    let last = lines[lines.len().saturating_sub(5)..].join(" / ");
    let mut chars = last.chars();
    let kept: String = chars.by_ref().take(400).collect();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

/// Whether a helper git2's credential helper would run for `url` is `!` with nothing after it:
/// git2 splits such a helper into no program at all and unwraps the first, where `sh` cannot
/// start, and a release build aborts on the panic. The keys are the ones git2 reads, in its
/// order: `credential.<url>.helper`, `credential.<scheme>://<domain>.helper` as the `url` crate
/// reads the address, and `credential.helper`, each its last value.
fn helper_runs_nothing(config: &git2::Config, url: &str) -> bool {
    let mut keys = vec![format!("credential.{url}.helper")];
    if let Ok(parsed) = url::Url::parse(url)
        && let Some(url::Host::Domain(host)) = parsed.host()
    {
        keys.push(format!("credential.{}://{host}.helper", parsed.scheme()));
    }
    keys.push("credential.helper".to_string());
    keys.iter().any(|key| {
        config.get_string(key).is_ok_and(|helper| {
            helper
                .strip_prefix('!')
                .is_some_and(|rest| rest.trim().is_empty())
        })
    })
}

/// Whether the host of an address libgit2 hands to SSH as `scheme://`, as it percent-decodes
/// it, is UTF-8. libgit2 takes the host after the last `@` of the authority, and decodes no
/// address scp writes, nor a user name into a host; any other address passes.
fn ssh_host_decodes_to_utf8(url: &str) -> bool {
    let Some(rest) = ssh_url(url) else {
        return true;
    };
    // Its authority runs to the first `/`, a `?` or `#` before it included.
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let mut bytes = Vec::new();
    let mut rest = host.as_bytes();
    while let Some((&first, after)) = rest.split_first() {
        let decoded = (first == b'%')
            .then(|| after.get(..2))
            .flatten()
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match decoded {
            Some(byte) => {
                bytes.push(byte);
                rest = &after[2..];
            }
            None => {
                bytes.push(first);
                rest = after;
            }
        }
    }
    std::str::from_utf8(&bytes).is_ok()
}

/// The address of the remote `name` as its configuration spells it, refused where it is not
/// UTF-8: libgit2 would connect to the address as it stands while every check here read another.
fn address(bytes: &[u8], name: &str) -> Result<String> {
    match std::str::from_utf8(bytes) {
        Ok(url) => Ok(url.to_string()),
        Err(_) => bail!("the address of {name} is not UTF-8, which Ocelli does not take: use git"),
    }
}

/// The scheme of `url`, lowercased, and what follows its `://`, where libgit2 reads it as an
/// address by its scheme: one of letters, digits, `+`, `-`, and `.` only. None for any other
/// address, one scp writes or a path.
fn scheme_of(url: &str) -> Option<(String, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let is_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    is_scheme.then(|| (scheme.to_ascii_lowercase(), rest))
}

/// What follows the `://` of `url` where libgit2 hands it to SSH by its scheme: every scheme but
/// http, https, git, and file.
fn ssh_url(url: &str) -> Option<&str> {
    scheme_of(url)
        .filter(|(scheme, _)| !matches!(scheme.as_str(), "http" | "https" | "git" | "file"))
        .map(|(_, rest)| rest)
}

/// Whether `url` reaches its remote over SSH: by its scheme as [`ssh_url`] reads it, or the
/// scp-like `host:path`.
fn is_ssh(url: &str) -> bool {
    scheme_of(url).is_none() || ssh_url(url).is_some()
}

/// The ways to authenticate, each tried once: the SSH agent, then the credential helper of the
/// developer's global and system configuration, never one the repository's own names (ADR-0035).
#[derive(Default)]
struct Tried {
    username: bool,
    agent: bool,
    helper: bool,
}

impl Tried {
    fn credentials(
        &mut self,
        url: &str,
        user: Option<&str>,
        allowed: git2::CredentialType,
    ) -> std::result::Result<git2::Cred, git2::Error> {
        use git2::CredentialType as Kind;
        let refuse = |why: &str| {
            Err(git2::Error::new(
                ErrorCode::Auth,
                git2::ErrorClass::Net,
                why,
            ))
        };
        // A line break in the address or the user name, both of which the repository's
        // configuration chooses, would add lines of its own to what a credential helper reads,
        // and could have it hand over the credential of another host (as CVE-2020-5260 did).
        if [url, user.unwrap_or_default()]
            .iter()
            .any(|s| s.contains(['\n', '\r', '\0']))
        {
            return refuse(
                "the remote's address holds a line break, which could pass a credential to another host",
            );
        }
        if allowed.contains(Kind::USERNAME) && !self.username {
            self.username = true;
            return git2::Cred::username(&user.map(str::to_string).unwrap_or_else(local_user));
        }
        if allowed.contains(Kind::SSH_KEY) && !self.agent {
            self.agent = true;
            let user = user.map(str::to_string).unwrap_or_else(local_user);
            return git2::Cred::ssh_key_from_agent(&user);
        }
        if allowed.contains(Kind::USER_PASS_PLAINTEXT) && !self.helper {
            self.helper = true;
            if let Err(why) = helper_may_answer_for(url) {
                return refuse(why);
            }
            let config = developers_config()?;
            if helper_runs_nothing(&config, url) {
                return refuse(
                    "a credential.helper of your configuration is '!' with nothing after it, \
                     which Ocelli does not run",
                );
            }
            return git2::Cred::credential_helper(&config, url, user);
        }
        refuse(
            "neither the SSH agent nor a credential helper of your configuration gave credentials \
             the server accepted",
        )
    }
}

/// The user name ssh would use where the address names none: the local one.
fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "git".to_string())
}

/// The developer's own configuration, global and system, without the repository's: where a
/// credential helper is looked up (ADR-0035).
pub(crate) fn developers_config() -> std::result::Result<git2::Config, git2::Error> {
    git2::Config::open_default()
}

/// Refuses an address a credential helper could answer for another host than the one libgit2
/// sends its answer to. A password goes over https only. git2 tells the helper the host as the
/// `url` crate reads it, and tells it none at all for an IP address, where a helper could match any
/// credential it holds; libgit2 connects to the host after the last `@` of the address. The two
/// must name one and the same domain, written plainly.
fn helper_may_answer_for(address: &str) -> std::result::Result<(), &'static str> {
    let scheme = "https://";
    if !address
        .get(..scheme.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
    {
        return Err("credentials are sent over https only, and this remote is not");
    }
    let parsed = url::Url::parse(address).map_err(|_| "the remote's address cannot be read")?;
    let Some(url::Host::Domain(host)) = parsed.host() else {
        return Err("credentials go only to a remote named by its domain, not by an IP address");
    };
    let authority = address[scheme.len()..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let named = authority.rsplit('@').next().unwrap_or_default();
    let named = match named.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => named,
    };
    let unusual = authority.contains(['\\', ' ']) || named.contains('%');
    if unusual || !named.eq_ignore_ascii_case(host) {
        return Err("the remote's address names its host in a way that could be read two ways");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test writes and removes a configuration file of its own"
    )]
    // Verifies: ADR-0035
    fn a_credential_helper_that_runs_nothing_is_found_where_git2_would_run_it() {
        let file = std::env::temp_dir().join(format!("ocelli-helper-{}", std::process::id()));
        let url = "https://e.org/r.git";
        let runs_nothing = |text: &str| {
            std::fs::write(&file, text).unwrap();
            let found = super::helper_runs_nothing(&git2::Config::open(&file).unwrap(), url);
            let _ = std::fs::remove_file(&file);
            found
        };
        assert!(runs_nothing("[credential]\n\thelper = \"! \"\n"));
        assert!(runs_nothing(
            "[credential \"https://e.org\"]\n\thelper = !\n"
        ));
        assert!(runs_nothing(
            "[credential \"https://e.org/r.git\"]\n\thelper = !\n"
        ));
        // The last value is the one read.
        assert!(!runs_nothing(
            "[credential]\n\thelper = !\n\thelper = store\n"
        ));
        // Another host's helper is not run for this one.
        assert!(!runs_nothing(
            "[credential \"https://other.org\"]\n\thelper = !\n"
        ));
        assert!(!runs_nothing("[credential]\n\thelper = !cat\n"));
        assert!(!runs_nothing("[credential]\n\thelper\n"));
    }

    #[test]
    // Verifies: ADR-0035
    fn the_host_of_an_ssh_address_is_read_decoded_as_libgit2_reads_it() {
        use super::ssh_host_decodes_to_utf8 as utf8;
        assert!(utf8("ssh://git@example.org/r.git"));
        assert!(utf8("ssh://git@ex%41mple.org/r.git"));
        assert!(!utf8("ssh://git@ex%ffmple.org/r.git"));
        assert!(!utf8("SSH+GIT://h%c3/r"));
        // A user name is no host. An IPv6 address with a zone passes here; libgit2 refuses it as
        // a malformed host before decoding anything, `%` being none of the characters it takes
        // between the brackets. Scp-like addresses and local paths are decoded by no one.
        assert!(utf8("ssh://us%ffer@example.org/r.git"));
        assert!(utf8("ssh://[fe80::1%25eth0]/r.git"));
        assert!(utf8("git@ex%ffmple.org:r.git"));
        assert!(utf8("C:\\repos\\a%ff"));
        assert!(utf8("ssh://h/pa%ffth"));
        // The authority runs to the first `/`, and every scheme but http, https, git, and file
        // goes to SSH.
        assert!(!utf8("ssh://h?%ff.x/r"));
        assert!(!utf8("ssh://h#%ff.x/r"));
        assert!(!utf8("foo://h%ff.x:22/r"));
        assert!(utf8("https://h%ff.x/r"));
        assert!(utf8("fi le://h%ff/r"));
    }

    #[test]
    // Verifies: ADR-0035
    fn each_way_to_authenticate_is_tried_once_and_in_turn() {
        use git2::CredentialType as Kind;
        let message = |r: Result<git2::Cred, git2::Error>| r.err().map(|e| e.message().to_string());
        // The user name tried, the next is the helper, which a plain address does not reach.
        let mut tried = super::Tried {
            username: true,
            agent: false,
            helper: false,
        };
        let answer = tried.credentials(
            "http://example.org/repo.git",
            Some("u"),
            Kind::USERNAME | Kind::USER_PASS_PLAINTEXT,
        );
        assert!(message(answer).is_some_and(|m| m.contains("https only")));
        // The agent tried, and no password asked for: nothing is left to try.
        let mut tried = super::Tried {
            username: true,
            agent: true,
            helper: false,
        };
        let answer = tried.credentials("ssh://git@example.org/r.git", Some("git"), Kind::SSH_KEY);
        assert!(message(answer).is_some_and(|m| m.contains("neither the SSH agent")));
        // The helper asked once already: it is not asked again.
        let mut tried = super::Tried {
            username: true,
            agent: true,
            helper: true,
        };
        let answer = tried.credentials(
            "https://example.org/repo.git",
            Some("u"),
            Kind::USER_PASS_PLAINTEXT,
        );
        assert!(message(answer).is_some_and(|m| m.contains("neither the SSH agent")));
    }

    #[test]
    // Verifies: ADR-0035
    fn an_address_is_ssh_by_its_scheme_or_as_scp_writes_it() {
        for ssh in [
            "ssh://git@example.org/r.git",
            "git+ssh://example.org/r.git",
            "git@example.org:r.git",
        ] {
            assert!(super::is_ssh(ssh), "{ssh}");
        }
        for other in ["https://example.org/r.git", "file:///srv/r.git"] {
            assert!(!super::is_ssh(other), "{other}");
        }
    }

    #[test]
    // Verifies: ADR-0035, Q-6
    fn a_line_break_in_the_user_name_is_refused_before_a_helper_is_asked() {
        use git2::CredentialType as Kind;
        for user in ["victim\nhost=evil.example", "victim\rx", "victim\0x"] {
            let mut tried = super::Tried {
                username: true,
                agent: true,
                helper: false,
            };
            let refused = tried
                .credentials(
                    "https://example.org/repo.git",
                    Some(user),
                    Kind::USER_PASS_PLAINTEXT,
                )
                .err()
                .map(|e| e.message().to_string());
            assert!(
                refused.as_deref().is_some_and(|m| m.contains("line break")),
                "{user:?}: {refused:?}"
            );
            assert!(!tried.helper, "{user:?}: a helper was asked");
        }
    }

    use super::helper_may_answer_for as asks;
    use super::{decode_base64, revoked_in, ssh_host, vouched_elsewhere};

    #[test]
    // Verifies: ADR-0035, Q-6
    fn a_helper_answers_only_for_a_plainly_named_domain_over_https() {
        for fine in [
            "https://example.org/repo.git",
            "https://user@Example.org:8443/repo.git",
            "HTTPS://example.org/a@b/repo.git",
            // Both parsers read the host after the last `@`, so the helper is asked about the
            // very host the answer goes to.
            "https://ex%61mple.org@evil.example/repo.git",
        ] {
            assert_eq!(asks(fine), Ok(()), "{fine}");
        }
        for (refused, why) in [
            ("http://example.org/repo.git", "https only"),
            ("https://192.0.2.1/repo.git", "IP address"),
            ("https://[2001:db8::1]/repo.git", "IP address"),
            ("https://%65xample.org/repo.git", "two ways"),
            ("https://example.org\\@evil.example/repo.git", "two ways"),
            ("https://bücher.example/repo.git", "two ways"),
            ("https://a b@example.org/repo.git", "two ways"),
        ] {
            let error = asks(refused).unwrap_err();
            assert!(error.contains(why), "{refused}: {error}");
        }
    }

    #[test]
    // Verifies: ADR-0035
    fn a_host_key_a_revoked_line_lists_is_refused_whatever_its_host_pattern() {
        let key = b"ssh-ed25519 key bytes";
        let encoded = "c3NoLWVkMjU1MTkga2V5IGJ5dGVz";
        assert_eq!(decode_base64(encoded).as_deref(), Some(&key[..]));
        let revoked = format!("@revoked * ssh-ed25519 {encoded}\n");
        assert!(revoked_in(&revoked, key));
        assert!(
            !revoked_in(&format!("example.org ssh-ed25519 {encoded}\n"), key),
            "trusted, not revoked"
        );
        assert!(
            !revoked_in("@revoked * ssh-ed25519 AAAA\n", key),
            "another key"
        );
        assert!(
            !revoked_in("@revoked * ssh-ed25519 !!!\n", key),
            "no base64 at all"
        );
    }

    #[test]
    // Verifies: ADR-0035
    fn a_refusal_names_where_else_a_host_is_vouched_for() {
        let line = "git.example.org ssh-ed25519 AAAA";
        assert_eq!(
            vouched_elsewhere("", line, "git.example.org"),
            Some("listed only in /etc/ssh/ssh_known_hosts")
        );
        let ca = "@cert-authority *.example.org,git.example.org ssh-ed25519 AAAA";
        assert_eq!(
            vouched_elsewhere(ca, "", "git.example.org"),
            Some("vouched for only by a @cert-authority line")
        );
        assert_eq!(vouched_elsewhere("", "", "git.example.org"), None);
        // A comment, a revoked line, and a certificate authority for another host vouch for none.
        assert_eq!(
            vouched_elsewhere("", "# git.example.org ssh-ed25519 AAAA", "git.example.org"),
            None
        );
        assert_eq!(
            vouched_elsewhere(
                "",
                "@revoked git.example.org ssh-ed25519 AAAA",
                "git.example.org"
            ),
            None
        );
        assert_eq!(
            vouched_elsewhere(
                "@cert-authority other.example ssh-ed25519 AAAA",
                "",
                "git.example.org"
            ),
            None
        );
        for (url, host) in [
            ("ssh://git@git.example.org:2222/r.git", "git.example.org"),
            ("git@git.example.org:r.git", "git.example.org"),
            ("git.example.org:r.git", "git.example.org"),
            ("SSH+GIT://u@git.example.org:22/r", "git.example.org"),
            ("foo://git.example.org/r", "git.example.org"),
        ] {
            assert_eq!(ssh_host(url), host, "{url}");
        }
    }

    #[test]
    // Verifies: ADR-0035
    fn every_scheme_libgit2_hands_to_ssh_is_ssh_here_too() {
        for ssh in [
            "SSH://h/r",
            "foo://h/r",
            "ssh+git://h/r",
            "h:r",
            "/srv/r.git",
        ] {
            assert!(super::is_ssh(ssh), "{ssh}");
        }
        for other in ["HTTPS://h/r", "http://h/r", "git://h/r", "FILE:///r"] {
            assert!(!super::is_ssh(other), "{other}");
        }
    }

    #[test]
    // Verifies: ADR-0035
    fn a_unc_path_is_spelled_as_libgit2_spells_a_remote_it_makes() {
        use super::as_libgit2_makes_it as made;
        assert_eq!(
            made("\\\\server\\share\\r.git", true),
            "//server/share/r.git"
        );
        assert_eq!(
            made("\\\\server\\share\\r.git", false),
            "\\\\server\\share\\r.git"
        );
        assert_eq!(made("C:\\r.git", true), "C:\\r.git");
        assert_eq!(made("\\\\?\\C:\\r.git", true), "\\\\?\\C:\\r.git");
        assert_eq!(made("ssh://h/r", true), "ssh://h/r");
    }

    #[test]
    // Verifies: ADR-0035
    fn what_a_remote_says_loses_escapes_and_every_invisible_character() {
        use super::cleaned;
        assert_eq!(
            cleaned("a\u{1b}[1;31mb\u{1b}]0;t\u{7}c\u{1b}]0;t\u{1b}\\d"),
            "abcd"
        );
        let invisible = "\u{ad}\u{34f}\u{61c}\u{115f}\u{1160}\u{200b}\u{202e}\u{2066}\u{3164}\
                         \u{fe0f}\u{feff}\u{ffa0}\u{1d173}\u{e0041}\u{e0100}";
        assert_eq!(cleaned(&format!("x{invisible}y")), "xy");
        assert_eq!(cleaned("one\u{2028}two\r\nthree"), "one / two / three");
        assert_eq!(cleaned("1\n2\n3\n4\n5\n6\n"), "2 / 3 / 4 / 5 / 6");
        assert_eq!(cleaned(&"z".repeat(500)).chars().count(), 401);
        assert_eq!(
            cleaned("remote error: caf\u{e9}"),
            "remote error: caf\u{e9}"
        );
    }
}
