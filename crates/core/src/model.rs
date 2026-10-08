use serde::{Deserialize, Serialize};

/// The most commits one chunk of a load holds (ADR-0016); the history loads chunk by chunk.
pub const CHUNK_COMMITS: usize = 3000;

/// Synthetic SHA used for the working-directory ("uncommitted changes") node.
/// It is not a real object id; callers compare against it to diff the workdir.
pub const WORKDIR_SHA: &str = "WORKING_DIRECTORY";

/// Distinguishes regular commits from the synthetic nodes we surface in the
/// graph (a stash entry, or the working-directory "uncommitted changes" node).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CommitKind {
    Commit,
    Stash,
    Working, // uncommitted working-directory changes (no real OID)
}

/// Raw commit data as loaded from git2, before any layout is assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitData {
    pub sha: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64, // Unix epoch seconds — when the change was written
    pub committer_name: String,
    pub committer_email: String,
    pub commit_time: i64, // Unix epoch seconds — when the commit was created (drives layout/sort)
    pub refs: Vec<String>,
    pub parent_shas: Vec<String>,
    pub kind: CommitKind,
    /// A stash node's place `n` in the stash, `stash@{n}` (ADR-0024); `None` for every other node.
    pub stash: Option<usize>,
}

/// A connection between a child commit and one of its parents.
/// `is_branch` marks a BRANCH edge (the first parent, staying in one column);
/// otherwise it is a MERGE edge, which crosses columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source_row: i32,
    pub source_col: i32,
    pub target_row: i32,
    pub target_col: i32,
    pub is_branch: bool,
    pub min_row: i32,
    pub max_row: i32,
    /// The lane a merge out of the main line runs up between its ends (ADR-0030); none for every
    /// other edge, and then left out of the JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<i32>,
}

/// One straight run of an edge as drawn: along a row between two lanes, or down a lane between
/// two rows, `from` never past `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Along { row: i32, from: i32, to: i32 },
    Down { lane: i32, from: i32, to: i32 },
}

impl Edge {
    /// The corners the edge is drawn through, from the parent to the child, as rows and lanes,
    /// after its type (ADR-0030): a branch edge along the parent's row and up the child's lane, a
    /// merge edge up the parent's lane and along the child's row, a merge out of the main line
    /// along the parent's row to its track, up the track, and along the child's row, and an edge
    /// within one lane down that lane alone. A corner that would repeat the one before is left out.
    pub fn path(&self) -> Vec<(i32, i32)> {
        let (sr, sc, tr, tc) = (
            self.source_row,
            self.source_col,
            self.target_row,
            self.target_col,
        );
        let corners = if let Some(track) = self.track {
            vec![(sr, sc), (sr, track), (tr, track), (tr, tc)]
        } else if self.is_branch {
            vec![(sr, sc), (sr, tc), (tr, tc)]
        } else {
            vec![(sr, sc), (tr, sc), (tr, tc)]
        };
        let mut path: Vec<(i32, i32)> = Vec::with_capacity(corners.len());
        for corner in corners {
            if path.last() != Some(&corner) {
                path.push(corner);
            }
        }
        path
    }

    /// The runs the edge is drawn with, one between each two corners of its path.
    pub fn route(&self) -> Vec<Run> {
        self.path()
            .windows(2)
            .map(|pair| {
                let ((r1, l1), (r2, l2)) = (pair[0], pair[1]);
                if r1 == r2 {
                    Run::Along {
                        row: r1,
                        from: l1.min(l2),
                        to: l1.max(l2),
                    }
                } else {
                    Run::Down {
                        lane: l1,
                        from: r1.min(r2),
                        to: r1.max(r2),
                    }
                }
            })
            .collect()
    }

    /// The lanes the edge reaches: one past the rightmost lane of its route.
    pub fn lanes(&self) -> usize {
        let rightmost = self
            .source_col
            .max(self.target_col)
            .max(self.track.unwrap_or(0));
        rightmost as usize + 1
    }
}

/// The serializable form of a node sent to the frontend.
/// The camelCase field names are the contract the canvas renderer reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeJson {
    pub row: i32,
    pub col: i32,
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author: String,
    pub author_date: String,
    pub author_avatar: String, // Gravatar URL (empty if no email); 404s when no avatar exists
    pub committer: String,
    pub date: String, // committer date
    pub committer_avatar: String,
    pub refs: Vec<String>,
    pub kind: CommitKind,
    /// A stash node's place in the stash, which a stash write names; absent for every other node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash: Option<usize>,
    pub parents: Vec<String>, // parent SHAs in commit order (first parent first); may include SHAs outside the loaded set
    pub children: Vec<String>, // SHAs of loaded children, newest (lowest row) first
}

/// The complete laid-out graph: every node with its row and column, plus all edges.
#[derive(Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<NodeJson>,
    pub edges: Vec<Edge>,
}

// ── Repository sidebar (ADR-0026) ───────────────────────────────────────────
// What the sidebar lists, read in the same pass as the history. These are refs
// and checkout locations — repository state, not history — so none of them is a
// node in the commit graph; each only points at the commit it resolves to.

/// A local branch and where it stands relative to its configured upstream.
/// `ahead`/`behind` are computed from refs already on disk and are therefore as
/// current as the last fetch — the viewer never fetches (specification Non-Goals).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    pub name: String,
    pub sha: String,
    pub short_sha: String,
    /// True for the branch `HEAD` currently points at.
    pub is_head: bool,
    /// The upstream's full name (e.g. `origin/main`), when one is configured.
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Whether every commit of the branch is in its upstream, or in `HEAD` where it has none, as
    /// `git branch -d` asks before deleting it.
    pub merged: bool,
}

/// A remote-tracking branch, named without its remote prefix (`feature/x`, not
/// `origin/feature/x`) because the remote it belongs to already groups it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    pub name: String,
    pub sha: String,
    pub short_sha: String,
}

/// A configured remote together with its tracking branches.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub branches: Vec<RemoteBranch>,
}

/// A working tree: the main one, plus every linked worktree.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub name: String,
    pub path: String,
    /// The checked-out branch, absent when the worktree has a detached `HEAD`.
    pub branch: Option<String>,
    pub sha: String,
    pub short_sha: String,
    /// True for the repository's main working tree, which `git2` does not
    /// report as a worktree of its own.
    pub is_main: bool,
    /// True for the working tree currently being viewed — selecting it would
    /// switch to where the user already is (ADR-0027).
    pub is_current: bool,
}

/// How a submodule's working copy stands to the commit the superproject pins.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubmoduleState {
    /// Configured, but no working copy on disk. Nothing to open, and the viewer
    /// never clones one — that would be both a write and egress (ADR-0023).
    Uninitialized,
    /// Checked out at a different commit than the superproject records.
    Modified,
    /// Checked out at exactly the pinned commit.
    InSync,
}

/// A submodule of this repository: a pinned commit in a *different* object
/// database, which is why it is opened rather than revealed (ADR-0027).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submodule {
    /// The name from `.gitmodules`, which may differ from the path.
    pub name: String,
    /// Path relative to the superproject's working directory.
    pub path: String,
    /// Absolute path of the working copy, empty when uninitialized.
    pub workdir: String,
    pub url: Option<String>,
    /// The commit the superproject pins, empty when it records none.
    pub sha: String,
    pub short_sha: String,
    /// The commit actually checked out, when there is a working copy.
    pub checked_out_short_sha: Option<String>,
    pub state: SubmoduleState,
}

/// What kind of repository the current one sits inside.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ParentKind {
    /// The superproject that configures this repository as a submodule.
    Superproject,
    /// The main working tree, when a linked worktree is being viewed.
    MainWorktree,
}

/// The repository one level up, so the sidebar can navigate back out of a
/// submodule or a linked worktree (ADR-0027). Read from git rather than
/// remembered by the UI, so the way back exists however the user arrived.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentRepo {
    pub kind: ParentKind,
    /// Directory name, for the row's label.
    pub name: String,
    pub path: String,
}

/// A tag, peeled so an annotated tag resolves to the commit it marks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub name: String,
    pub sha: String,
    pub short_sha: String,
    /// Whether the tag is a tag object with a message of its own, rather than a bare reference.
    pub annotated: bool,
}

/// Everything the repository sidebar lists for one repository read.
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepoRefs {
    pub locals: Vec<LocalBranch>,
    pub remotes: Vec<Remote>,
    pub worktrees: Vec<Worktree>,
    pub tags: Vec<Tag>,
    pub submodules: Vec<Submodule>,
    /// Absent when this repository is neither a submodule nor a linked worktree.
    pub parent: Option<ParentRepo>,
    /// The remote a fetch without one named reaches: the remote of the checked-out branch's
    /// upstream, else `origin`, else the only remote there is.
    pub default_remote: Option<String>,
    /// The operation stopped in the repository, whoever started it, with its conflicted files
    /// (ADR-0033).
    pub in_progress: Option<InProgress>,
}

impl RepoRefs {
    /// The checked-out branch, where one is.
    pub fn head(&self) -> Option<&LocalBranch> {
        self.locals.iter().find(|b| b.is_head)
    }
}

/// A merge, rebase, cherry-pick, or revert stopped in the repository, as `git status` reads it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InProgress {
    pub operation: Operation,
    pub conflicts: Vec<Conflict>,
    /// The commit `HEAD` is at, which continuing and aborting expect to find.
    pub head: String,
}

/// An operation that can stop on a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    /// Its name as `git` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        }
    }
}

/// A conflicted file: its path and the blob ids of the index's stages, the common ancestor's,
/// ours, and theirs, each absent where that side has no file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub path: String,
    pub ancestor: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    Add,
    Delete,
    Context,
    Hunk,
    Meta,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

/// How much of a file's uncommitted change the index holds (G-4).
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Staged {
    No,
    Yes,
    /// The index holds a change, and the working directory changes the file further.
    Partly,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>, // only set for renames
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    /// For the working-directory node's files only, each of which carries one: how the file
    /// stands in the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staging: Option<Staging>,
}

/// How a file of the working-directory node stands in the index: how much of its change is
/// staged, and the content shown, which staging and unstaging expect to find (ADR-0033).
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Staging {
    pub staged: Staged,
    pub shown: Shown,
}

/// A file's content in the working directory and in the index, by blob id; `None` where it has
/// none there.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub workdir: Option<String>,
    pub index: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(source: (i32, i32), target: (i32, i32), is_branch: bool, track: Option<i32>) -> Edge {
        Edge {
            source_row: source.0,
            source_col: source.1,
            target_row: target.0,
            target_col: target.1,
            is_branch,
            min_row: source.0.min(target.0),
            max_row: source.0.max(target.0),
            track,
        }
    }

    #[test]
    // Verifies: ADR-0030
    fn each_edge_type_takes_its_route() {
        let along = |row, from, to| Run::Along { row, from, to };
        let down = |lane, from, to| Run::Down { lane, from, to };
        // A branch edge: along the parent's row, then up the child's lane, either side.
        assert_eq!(
            edge((5, 0), (2, 2), true, None).route(),
            [along(5, 0, 2), down(2, 2, 5)]
        );
        assert_eq!(
            edge((5, 2), (2, 0), true, None).route(),
            [along(5, 0, 2), down(0, 2, 5)]
        );
        // A merge edge: up the parent's lane, then along the child's row, either side.
        assert_eq!(
            edge((5, 1), (2, 3), false, None).route(),
            [down(1, 2, 5), along(2, 1, 3)]
        );
        // Within one lane: down that lane alone.
        assert_eq!(edge((5, 1), (2, 1), true, None).route(), [down(1, 2, 5)]);
        // A merge out of the main line: along to its track, up the track, along to the child.
        assert_eq!(
            edge((5, 0), (2, 1), false, Some(3)).route(),
            [along(5, 0, 3), down(3, 2, 5), along(2, 1, 3)]
        );
        // The track in the child's lane: up it to the child, no second run along.
        assert_eq!(
            edge((5, 0), (2, 2), false, Some(2)).route(),
            [along(5, 0, 2), down(2, 2, 5)]
        );
        // Down to a late child below its parent: the same runs, the rows the other way.
        assert_eq!(edge((1, 0), (4, 0), true, None).route(), [down(0, 1, 4)]);
    }

    #[test]
    // Verifies: ADR-0030
    fn an_edge_runs_through_its_corners_from_the_parent_to_the_child() {
        assert_eq!(
            edge((5, 0), (2, 2), true, None).path(),
            [(5, 0), (5, 2), (2, 2)]
        );
        assert_eq!(
            edge((5, 1), (2, 3), false, None).path(),
            [(5, 1), (2, 1), (2, 3)]
        );
        assert_eq!(edge((5, 1), (2, 1), false, None).path(), [(5, 1), (2, 1)]);
        assert_eq!(
            edge((5, 0), (2, 1), false, Some(3)).path(),
            [(5, 0), (5, 3), (2, 3), (2, 1)]
        );
        assert_eq!(
            edge((5, 0), (2, 2), false, Some(2)).path(),
            [(5, 0), (5, 2), (2, 2)]
        );
    }

    #[test]
    // Verifies: ADR-0030, ADR-0031
    fn an_edge_reaches_one_lane_past_its_rightmost() {
        assert_eq!(edge((5, 0), (2, 2), true, None).lanes(), 3);
        assert_eq!(edge((5, 0), (2, 1), false, Some(4)).lanes(), 5);
    }
}
