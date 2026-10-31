//! The transfers of the checked-out branch, decided once for both front ends (ADR-0035): which
//! fetch, pull, and push the refs a front end holds make possible, or why there is none, and
//! which write is a transfer and which of those can be stopped while it runs. The desktop host
//! sends the answers to the web interface as `Transfers`, which it restates none of (ADR-0036).

use crate::model::RepoRefs;
use crate::port::Write;
use serde::Serialize;

/// A transfer the refs make possible, with whether it can be stopped while it runs, or why there
/// is none: `{ write, canStop }` or `{ reason }` in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Transfer {
    Offered {
        write: Write,
        #[serde(rename = "canStop")]
        can_stop: bool,
    },
    Refused {
        reason: String,
    },
}

impl From<Result<Write, String>> for Transfer {
    fn from(answer: Result<Write, String>) -> Self {
        match answer {
            Ok(write) => Transfer::Offered {
                can_stop: write.can_stop(),
                write,
            },
            Err(reason) => Transfer::Refused { reason },
        }
    }
}

/// The fetch, pull, and push the refs make possible, as a toolbar offers them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Transfers {
    pub fetch: Transfer,
    pub pull: Transfer,
    pub push: Transfer,
}

impl RepoRefs {
    /// The fetch, pull, and push of the refs, each with whether it can be stopped.
    pub fn transfers(&self) -> Transfers {
        Transfers {
            fetch: self.fetch().into(),
            pull: self.pull().into(),
            push: self.push().into(),
        }
    }

    /// The fetch of the default remote, or why there is none.
    pub fn fetch(&self) -> Result<Write, String> {
        let remote = self.default_remote.clone().ok_or("No remote to fetch")?;
        Ok(Write::Fetch { remote })
    }

    /// The pull of the checked-out branch from its upstream, or why there is none.
    pub fn pull(&self) -> Result<Write, String> {
        let head = self.head().ok_or("No branch is checked out to pull")?;
        if head.upstream.is_none() {
            return Err(format!("{} has no upstream to pull from", head.name));
        }
        Ok(Write::Pull {
            branch: head.name.clone(),
            tip: head.sha.clone(),
        })
    }

    /// The push of the checked-out branch to its upstream, or to the default remote under its
    /// own name, or why there is none.
    pub fn push(&self) -> Result<Write, String> {
        let head = self.head().ok_or("No branch is checked out to push")?;
        if head.upstream.is_none() && self.default_remote.is_none() {
            return Err(format!("There is no remote to push {} to", head.name));
        }
        Ok(Write::Push {
            branch: head.name.clone(),
            tip: head.sha.clone(),
        })
    }
}

impl Write {
    /// Whether the write is a transfer: it reaches a remote, tells its progress as it goes, and
    /// the repository is read again after it whether it was written or refused.
    pub fn is_transfer(&self) -> bool {
        matches!(
            self,
            Write::Fetch { .. } | Write::Pull { .. } | Write::Push { .. }
        )
    }

    /// Whether the write can be stopped while it runs: a fetch, and a pull while it fetches.
    /// libgit2 cannot stop a push once its objects are sent, and a local write is over at once.
    pub fn can_stop(&self) -> bool {
        matches!(self, Write::Fetch { .. } | Write::Pull { .. })
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{LocalBranch, RepoRefs};
    use crate::port::Write;

    fn refs(head: Option<Option<&str>>, default_remote: Option<&str>) -> RepoRefs {
        RepoRefs {
            locals: head
                .map(|upstream| LocalBranch {
                    name: "main".into(),
                    sha: "s".into(),
                    short_sha: "s".into(),
                    is_head: true,
                    upstream: upstream.map(str::to_string),
                    ahead: 0,
                    behind: 0,
                    merged: true,
                })
                .into_iter()
                .collect(),
            default_remote: default_remote.map(str::to_string),
            ..RepoRefs::default()
        }
    }

    #[test]
    // Verifies: ADR-0035, G-12
    fn a_pull_and_a_push_are_those_of_the_checked_out_branch_where_a_remote_takes_them() {
        let pull = Write::Pull {
            branch: "main".into(),
            tip: "s".into(),
        };
        let push = Write::Push {
            branch: "main".into(),
            tip: "s".into(),
        };
        let tracked = refs(Some(Some("origin/main")), Some("origin"));
        assert_eq!(tracked.pull(), Ok(pull));
        assert_eq!(tracked.push(), Ok(push.clone()));
        assert_eq!(
            tracked.fetch(),
            Ok(Write::Fetch {
                remote: "origin".into()
            })
        );
        let untracked = refs(Some(None), Some("origin"));
        assert_eq!(
            untracked.pull(),
            Err("main has no upstream to pull from".into())
        );
        assert_eq!(untracked.push(), Ok(push));
        let alone = refs(Some(None), None);
        assert_eq!(
            alone.push(),
            Err("There is no remote to push main to".into())
        );
        assert_eq!(alone.fetch(), Err("No remote to fetch".into()));
        let detached = refs(None, Some("origin"));
        assert_eq!(
            detached.pull(),
            Err("No branch is checked out to pull".into())
        );
        assert_eq!(
            detached.push(),
            Err("No branch is checked out to push".into())
        );
    }

    #[test]
    // Verifies: ADR-0035, G-17
    fn a_fetch_and_a_pull_can_be_stopped_a_push_and_a_local_write_cannot() {
        let fetch = Write::Fetch { remote: "o".into() };
        let pull = Write::Pull {
            branch: "b".into(),
            tip: "t".into(),
        };
        let push = Write::Push {
            branch: "b".into(),
            tip: "t".into(),
        };
        let local = Write::Detach { commit: "c".into() };
        assert!(fetch.can_stop() && pull.can_stop());
        assert!(!push.can_stop() && !local.can_stop());
        assert!(fetch.is_transfer() && pull.is_transfer() && push.is_transfer());
        assert!(!local.is_transfer());
    }

    #[test]
    // Verifies: ADR-0036, ADR-0035
    fn the_transfers_serialize_as_a_write_that_says_whether_it_stops_or_a_reason() {
        let tracked = refs(Some(Some("origin/main")), Some("origin")).transfers();
        assert_eq!(
            serde_json::to_value(&tracked).unwrap(),
            serde_json::json!({
                "fetch": { "write": { "kind": "fetch", "remote": "origin" }, "canStop": true },
                "pull": { "write": { "kind": "pull", "branch": "main", "tip": "s" }, "canStop": true },
                "push": { "write": { "kind": "push", "branch": "main", "tip": "s" }, "canStop": false },
            })
        );
        let detached = refs(None, None).transfers();
        assert_eq!(
            serde_json::to_value(&detached).unwrap(),
            serde_json::json!({
                "fetch": { "reason": "No remote to fetch" },
                "pull": { "reason": "No branch is checked out to pull" },
                "push": { "reason": "No branch is checked out to push" },
            })
        );
    }
}
