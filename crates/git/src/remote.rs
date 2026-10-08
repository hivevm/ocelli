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
    let url = remote.url().unwrap_or_default().to_string();
    let transfer = Transfer::new("Fetching", name, progress);
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
}

impl<'p> Transfer<'p> {
    fn new(doing: &'static str, name: &str, progress: &'p mut dyn FnMut(&str) -> bool) -> Self {
        set_server_timeouts();
        Self {
            doing,
            name: name.to_string(),
            progress: std::cell::RefCell::new(progress),
            cancelled: std::cell::Cell::new(false),
            tried: Default::default(),
            revoked: Default::default(),
        }
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
        callbacks.sideband_progress(|_| self.go_on("the server is preparing"));
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
                    e.message()
                )
            }
            ErrorCode::Auth => anyhow::anyhow!("{name} refused the credentials: {}", e.message()),
            ErrorCode::NotFastForward => {
                anyhow::anyhow!(
                    "{name} has commits the branch does not: the remote is ahead, so pull first"
                )
            }
            _ => e.into(),
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
    transaction
        .lock_ref(&refname)
        .with_context(|| format!("'{name}' is locked by another program: try again later"))?;
    branch_as_shown(repo, name, tip)?;
    check_out(repo, &target)?;
    transaction.set_target(&refname, theirs, None, "pull: Fast-forward")?;
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
    let url = remote
        .pushurl()
        .or(remote.url())
        .unwrap_or_default()
        .to_string();
    let transfer = Transfer::new("Pushing", &remote_name, progress);
    let refused = std::cell::RefCell::new(None);
    let mut callbacks = transfer.callbacks();
    callbacks.push_update_reference(|_, status| {
        if let Some(why) = status {
            *refused.borrow_mut() = Some(why.to_string());
        }
        Ok(())
    });
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    // No `+`: a push that is not a fast-forward is the remote's to refuse.
    let pushed = remote.push(&[format!("{refname}:{destination}")], Some(&mut options));
    drop(options);
    pushed.map_err(|e| transfer.refusal(e, &url, "pushing to"))?;
    if let Some(why) = refused.into_inner() {
        bail!("{remote_name} refused '{name}' ({why}): the remote is ahead, so pull first");
    }
    if first {
        repo.find_branch(name, BranchType::Local)?
            .set_upstream(Some(&format!("{remote_name}/{name}")))
            .with_context(|| format!("'{name}' was pushed, but its upstream could not be set"))?;
    }
    Ok(())
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

/// The host an SSH address names: `ssh://[user@]host[:port]/path` or `[user@]host:path`.
fn ssh_host(url: &str) -> &str {
    let rest = url
        .strip_prefix("ssh://")
        .or_else(|| url.strip_prefix("git+ssh://"))
        .unwrap_or(url);
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

/// Whether `url` reaches its remote over SSH: `ssh://`, or the scp-like `host:path`.
fn is_ssh(url: &str) -> bool {
    url.starts_with("ssh://") || url.starts_with("git+ssh://") || !url.contains("://")
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
            return git2::Cred::credential_helper(&developers_config()?, url, user);
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
    // Verifies: ADR-0035, Q-7
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
    // Verifies: ADR-0035, Q-7
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
        ] {
            assert_eq!(ssh_host(url), host, "{url}");
        }
    }
}
