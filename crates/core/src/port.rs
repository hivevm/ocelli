//! The repository ports (ADR-0009): what the core needs from a repository, owned by the core and
//! implemented by an adapter. Every path names a directory inside the repository.

use crate::model::{CommitData, DiffLine, FileChange, RepoRefs};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Reads a repository's history and state.
pub trait RepositoryReader {
    /// Loads the whole history in chunks of [`crate::model::CHUNK_COMMITS`] in walk order, the
    /// first with the stash and working-directory nodes added (ADR-0024), handing each to
    /// `on_chunk` as it is read; `on_chunk` returns whether to go on, so a new load session can cancel the walk
    /// (ADR-0016). `progress` receives one message per step.
    fn load_chunks(
        &self,
        repo: &Path,
        progress: &mut dyn FnMut(String),
        on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
    ) -> Result<()>;

    /// The tip of the main line (ADR-0032): the commit of the local branch `origin/HEAD` names,
    /// else of `main`, else of `master`, else the checked-out commit; none where `HEAD` does not
    /// resolve either.
    fn load_main_tip(&self, repo: &Path) -> Result<Option<String>>;

    /// Loads every ref category the repository sidebar shows.
    fn load_refs(&self, repo: &Path) -> Result<RepoRefs>;

    /// Loads the files a commit changed; [`crate::model::WORKDIR_SHA`] yields the uncommitted
    /// changes.
    fn load_commit_files(&self, repo: &Path, sha: &str) -> Result<Vec<FileChange>>;

    /// Loads the diff of one file of a commit; [`crate::model::WORKDIR_SHA`] yields the
    /// uncommitted diff.
    fn load_file_diff(&self, repo: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>>;
}

/// A write to a repository (ADR-0033), naming the state it expects to find: a write whose expected
/// state the repository no longer holds is refused with nothing changed. A front end out of process
/// hands it over as JSON, its kind in `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Write {
    /// Tags `commit` with `name`, which is expected to be free: annotated by the configured
    /// identity with `message` where one is given, lightweight where none is.
    CreateTag {
        name: String,
        commit: String,
        message: Option<String>,
    },
    /// Deletes the tag `name`, expected to point at `commit` and to be annotated or lightweight
    /// as `annotated` says.
    DeleteTag {
        name: String,
        commit: String,
        annotated: bool,
    },
    /// Puts the uncommitted changes aside as a stash with `message`, the untracked files with
    /// them where `untracked` says so; ignored files never.
    CreateStash {
        message: Option<String>,
        untracked: bool,
    },
    /// Applies the stash at `index`, expected to be `commit`, to the working directory, and drops
    /// it after where `drop` says so. A stash that would not apply cleanly is refused.
    ApplyStash {
        index: usize,
        commit: String,
        drop: bool,
    },
    /// Drops the stash at `index`, expected to be `commit`.
    DropStash { index: usize, commit: String },
    /// Stages the whole file at `path`, whose content in the working directory is expected to be
    /// `shown`, by blob id, `None` where it is deleted there.
    StageFile { path: String, shown: Option<String> },
    /// Takes the file at `path` out of the index, whose content there is expected to be `shown`,
    /// `None` where the index removes it; the working directory keeps its change.
    UnstageFile { path: String, shown: Option<String> },
    /// Commits the index with `message` on `head`, the commit expected to be checked out, `None`
    /// on a branch without a commit yet.
    Commit {
        message: String,
        head: Option<String>,
    },
    /// Creates the local branch `name`, expected to be free, at `commit`, and switches to it where
    /// `switch` says so.
    CreateBranch {
        name: String,
        commit: String,
        switch: bool,
    },
    /// Switches to the local branch `name`, expected at `tip`, taking the uncommitted changes
    /// along where the switch touches none of them.
    SwitchBranch { name: String, tip: String },
    /// Switches to `commit` with `HEAD` detached, the changes taken along as for a branch.
    Detach { commit: String },
    /// Creates the local branch `branch` tracking `remote`'s branch of that name, expected at
    /// `tip`, and switches to it, as `git switch <branch>` does.
    TrackRemote {
        remote: String,
        branch: String,
        tip: String,
    },
    /// Deletes the local branch `name`, expected at `tip`, and merged or not as `merged` says.
    DeleteBranch {
        name: String,
        tip: String,
        merged: bool,
    },
    /// Fetches `remote`, updating its remote branches (ADR-0035).
    Fetch { remote: String },
    /// Pulls the checked-out branch `branch`, expected at `tip`, from its upstream, fast-forward
    /// only (ADR-0035).
    Pull { branch: String, tip: String },
    /// Pushes the checked-out branch `branch`, expected at `tip`, to its upstream, or to the
    /// remote's branch of its name, set as its upstream, where it has none; never by force
    /// (ADR-0035).
    Push { branch: String, tip: String },
    /// Merges `commit`, named `name` in the message where it is a branch, into the checked-out
    /// branch `branch`, expected at `tip` (ADR-0033).
    Merge {
        branch: String,
        tip: String,
        commit: String,
        name: Option<String>,
    },
    /// Rebases the checked-out branch `branch`, expected at `tip`, onto `onto` (ADR-0033).
    Rebase {
        branch: String,
        tip: String,
        onto: String,
    },
    /// Applies `commit` onto the checked-out branch `branch`, expected at `tip`, and commits it
    /// (ADR-0033).
    CherryPick {
        branch: String,
        tip: String,
        commit: String,
    },
    /// Applies the inverse of `commit` onto the checked-out branch `branch`, expected at `tip`,
    /// and commits it (ADR-0033).
    Revert {
        branch: String,
        tip: String,
        commit: String,
    },
    /// Moves the checked-out branch `branch`, expected at `tip`, to `commit`, as `mode` says
    /// (ADR-0033).
    Reset {
        branch: String,
        tip: String,
        commit: String,
        mode: ResetMode,
    },
    /// Resolves the conflicted file `path`, whose conflict is expected to be `conflict`, as
    /// `take` says (ADR-0033).
    Resolve {
        path: String,
        conflict: Conflict,
        take: Take,
    },
    /// Continues the operation in progress, `HEAD` expected at `head`, once nothing conflicts
    /// (ADR-0033).
    Continue { operation: Operation, head: String },
    /// Aborts the operation in progress, back to where the developer was (ADR-0033).
    Abort { operation: Operation, head: String },
}

/// How a reset moves the checked-out branch, as `git reset` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetMode {
    /// The branch alone.
    Soft,
    /// The branch and the index.
    Mixed,
    /// The branch, the index, and the working directory.
    Hard,
}

/// Which side of a conflicted file a resolution takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Take {
    Ours,
    Theirs,
    /// The working directory's file, as the developer edited it.
    AsEdited,
}

pub use crate::model::{Conflict, Operation};

impl Write {
    /// What the write discards, in the words a confirmation names it with, or `None` for a write
    /// that discards nothing (ADR-0034).
    pub fn discards(&self) -> Option<String> {
        match self {
            Write::CreateTag { .. }
            | Write::CreateStash { .. }
            | Write::ApplyStash { drop: false, .. }
            | Write::StageFile { .. }
            | Write::UnstageFile { .. }
            | Write::Commit { .. }
            | Write::CreateBranch { .. }
            | Write::SwitchBranch { .. }
            | Write::Detach { .. }
            | Write::TrackRemote { .. }
            | Write::DeleteBranch { merged: true, .. }
            | Write::Fetch { .. }
            | Write::Pull { .. }
            | Write::Push { .. }
            | Write::Merge { .. }
            | Write::Rebase { .. }
            | Write::CherryPick { .. }
            | Write::Revert { .. }
            | Write::Reset {
                mode: ResetMode::Soft | ResetMode::Mixed,
                ..
            }
            | Write::Resolve {
                take: Take::AsEdited,
                ..
            }
            | Write::Continue { .. } => None,
            Write::DeleteTag {
                name,
                annotated: true,
                ..
            } => Some(format!("the tag {name} and its message")),
            Write::DeleteTag { name, .. } => Some(format!("the tag {name}")),
            Write::ApplyStash { index, .. } => Some(format!(
                "the stash stash@{{{index}}}, its changes kept only in the working directory"
            )),
            Write::DropStash { index, .. } => {
                Some(format!("the stash stash@{{{index}}} and the changes in it"))
            }
            Write::DeleteBranch { name, .. } => Some(format!(
                "the branch {name}, which is not merged, and the commits only it reaches"
            )),
            Write::Reset {
                mode: ResetMode::Hard,
                branch,
                ..
            } => Some(format!(
                "the uncommitted changes to {branch}'s files, and its commits after the one reset to"
            )),
            Write::Resolve { path, take, .. } => Some(format!(
                "{path} as the working directory has it, any edit to it included, for {} side",
                if *take == Take::Ours { "our" } else { "their" }
            )),
            Write::Abort { operation, .. } => Some(format!(
                "the {} in progress and every conflict resolved in it",
                operation.name()
            )),
        }
    }
}

/// Whether the developer confirmed a write (ADR-0034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// The developer was asked and said yes.
    Given,
    /// The developer was not asked.
    Absent,
}

/// Writes to a repository, running no program the repository names (ADR-0033).
pub trait RepositoryWriter {
    /// Performs `write`, or refuses it with nothing changed and says why. Returns the commit the
    /// write made, which the load after it selects, where it made one. A write that takes long,
    /// one that reaches a remote, tells `progress` how far it has come, and stops where `progress`
    /// returns false.
    fn write_reporting(
        &self,
        repo: &Path,
        write: &Write,
        progress: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Option<String>>;

    /// Performs `write` with no one watching its progress.
    fn write(&self, repo: &Path, write: &Write) -> Result<Option<String>> {
        self.write_reporting(repo, write, &mut |_| true)
    }
}

/// Hands `write` to `writer`, refusing a write that discards work before it reaches the
/// repository unless the developer has confirmed it (ADR-0034). Every front end writes through
/// here.
pub fn perform(
    writer: &impl RepositoryWriter,
    repo: &Path,
    write: &Write,
    confirmation: Confirmation,
    progress: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<String>> {
    if let Some(lost) = write.discards()
        && confirmation != Confirmation::Given
    {
        bail!("discarding {lost} needs the developer's confirmation");
    }
    writer.write_reporting(repo, write, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A writer that records what reaches it.
    #[derive(Default)]
    struct Recorder(RefCell<Vec<Write>>);

    impl RepositoryWriter for Recorder {
        fn write_reporting(
            &self,
            _: &Path,
            write: &Write,
            _: &mut dyn FnMut(&str) -> bool,
        ) -> Result<Option<String>> {
            self.0.borrow_mut().push(write.clone());
            Ok(None)
        }
    }

    fn create() -> Write {
        Write::CreateTag {
            name: "v1".into(),
            commit: "c".into(),
            message: None,
        }
    }

    fn delete(annotated: bool) -> Write {
        Write::DeleteTag {
            name: "v1".into(),
            commit: "c".into(),
            annotated,
        }
    }

    fn apply(drop: bool) -> Write {
        Write::ApplyStash {
            index: 1,
            commit: "s".into(),
            drop,
        }
    }

    #[test]
    // Verifies: ADR-0034, Q-6
    fn each_write_that_discards_work_names_what_it_discards() {
        assert_eq!(create().discards(), None);
        let stash = Write::CreateStash {
            message: None,
            untracked: true,
        };
        assert_eq!(stash.discards(), None);
        assert_eq!(apply(false).discards(), None);
        assert_eq!(
            apply(true).discards().as_deref(),
            Some("the stash stash@{1}, its changes kept only in the working directory")
        );
        let drop = Write::DropStash {
            index: 1,
            commit: "s".into(),
        };
        assert_eq!(
            drop.discards().as_deref(),
            Some("the stash stash@{1} and the changes in it")
        );
        let delete_branch = |merged| Write::DeleteBranch {
            name: "side".into(),
            tip: "s".into(),
            merged,
        };
        assert_eq!(delete_branch(true).discards(), None);
        let reset = |mode| Write::Reset {
            branch: "main".into(),
            tip: "t".into(),
            commit: "c".into(),
            mode,
        };
        assert_eq!(reset(ResetMode::Soft).discards(), None);
        assert_eq!(reset(ResetMode::Mixed).discards(), None);
        assert!(reset(ResetMode::Hard).discards().is_some());
        let abort = Write::Abort {
            operation: Operation::Rebase,
            head: "h".into(),
        };
        assert_eq!(
            abort.discards().as_deref(),
            Some("the rebase in progress and every conflict resolved in it")
        );
        let resolve = |take| Write::Resolve {
            path: "f".into(),
            conflict: Conflict {
                path: "f".into(),
                ancestor: None,
                ours: None,
                theirs: None,
            },
            take,
        };
        assert_eq!(resolve(Take::AsEdited).discards(), None);
        assert_eq!(
            resolve(Take::Theirs).discards().as_deref(),
            Some("f as the working directory has it, any edit to it included, for their side")
        );
        assert!(resolve(Take::Ours).discards().is_some());
        let carry_on = Write::Continue {
            operation: Operation::Rebase,
            head: "h".into(),
        };
        assert_eq!(carry_on.discards(), None);
        assert_eq!(
            delete_branch(false).discards().as_deref(),
            Some("the branch side, which is not merged, and the commits only it reaches")
        );
        assert_eq!(delete(false).discards().as_deref(), Some("the tag v1"));
        assert_eq!(
            delete(true).discards().as_deref(),
            Some("the tag v1 and its message")
        );
    }

    #[test]
    fn a_write_crosses_to_a_front_end_as_json_named_by_its_kind() {
        let json = r#"{"kind":"applyStash","index":1,"commit":"s","drop":true}"#;
        assert_eq!(serde_json::from_str::<Write>(json).unwrap(), apply(true));
        assert_eq!(serde_json::to_string(&apply(true)).unwrap(), json);
    }

    #[test]
    // Verifies: ADR-0034, Q-6
    fn a_write_that_discards_work_without_a_confirmation_never_reaches_the_repository() {
        let writer = Recorder::default();
        let repo = Path::new("/repo");
        let error = perform(
            &writer,
            repo,
            &delete(true),
            Confirmation::Absent,
            &mut |_| true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("confirmation"), "{error}");
        assert!(writer.0.borrow().is_empty());

        perform(
            &writer,
            repo,
            &delete(true),
            Confirmation::Given,
            &mut |_| true,
        )
        .unwrap();
        perform(&writer, repo, &create(), Confirmation::Absent, &mut |_| {
            true
        })
        .unwrap();
        assert_eq!(*writer.0.borrow(), [delete(true), create()]);
    }
}
