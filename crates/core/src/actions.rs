//! The operations of G-5 a commit offers on the checked-out branch, decided once for both front
//! ends (ADR-0033): which of merging it in, rebasing onto it, cherry-picking it, reverting it,
//! and resetting to it make sense, each with the write it hands the writer.

use crate::model::RepoRefs;
use crate::port::{ResetMode, Write};

impl RepoRefs {
    /// The operations the commit `sha` offers on the checked-out branch, in the order a front end
    /// lists them, or why there are none. The checked-out commit offers only a revert and a
    /// reset: merging it, rebasing onto it, or picking it changes nothing. A merge is named after
    /// a local branch at the commit other than the checked-out one, where there is one. A reset
    /// is mixed, as `git reset` is by default; a front end sets the mode the developer chose.
    pub fn history_actions(&self, sha: &str) -> Result<Vec<Write>, String> {
        let head = self
            .locals
            .iter()
            .find(|b| b.is_head)
            .ok_or("No branch is checked out")?;
        let (branch, tip, commit) = (head.name.clone(), head.sha.clone(), sha.to_string());
        let mut actions = Vec::new();
        if head.sha != sha {
            let name = self
                .locals
                .iter()
                .find(|b| b.sha == sha && !b.is_head)
                .map(|b| b.name.clone());
            actions.extend([
                Write::Merge {
                    branch: branch.clone(),
                    tip: tip.clone(),
                    commit: commit.clone(),
                    name,
                },
                Write::Rebase {
                    branch: branch.clone(),
                    tip: tip.clone(),
                    onto: commit.clone(),
                },
                Write::CherryPick {
                    branch: branch.clone(),
                    tip: tip.clone(),
                    commit: commit.clone(),
                },
            ]);
        }
        actions.extend([
            Write::Revert {
                branch: branch.clone(),
                tip: tip.clone(),
                commit: commit.clone(),
            },
            Write::Reset {
                branch,
                tip,
                commit,
                mode: ResetMode::Mixed,
            },
        ]);
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{LocalBranch, RepoRefs};
    use crate::port::Write;

    fn local(name: &str, sha: &str, is_head: bool) -> LocalBranch {
        LocalBranch {
            name: name.into(),
            sha: sha.into(),
            short_sha: sha.into(),
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            merged: true,
        }
    }

    fn kinds(actions: &[Write]) -> Vec<&'static str> {
        actions
            .iter()
            .map(|w| match w {
                Write::Merge { .. } => "merge",
                Write::Rebase { .. } => "rebase",
                Write::CherryPick { .. } => "cherry-pick",
                Write::Revert { .. } => "revert",
                Write::Reset { .. } => "reset",
                _ => "other",
            })
            .collect()
    }

    #[test]
    // Verifies: ADR-0033, G-5
    fn a_commit_offers_what_changes_the_checked_out_branch_and_a_merge_is_named_after_its_branch() {
        let refs = RepoRefs {
            locals: vec![local("main", "h", true), local("topic", "t", false)],
            ..RepoRefs::default()
        };
        let other = refs.history_actions("t").unwrap();
        assert_eq!(
            kinds(&other),
            ["merge", "rebase", "cherry-pick", "revert", "reset"]
        );
        assert!(matches!(&other[0], Write::Merge { name: Some(n), .. } if n == "topic"));
        let unnamed = refs.history_actions("x").unwrap();
        assert!(matches!(&unnamed[0], Write::Merge { name: None, .. }));
        assert_eq!(
            kinds(&refs.history_actions("h").unwrap()),
            ["revert", "reset"]
        );
        let detached = RepoRefs::default();
        assert_eq!(
            detached.history_actions("t"),
            Err("No branch is checked out".into())
        );
    }
}
