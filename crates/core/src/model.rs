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
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub name: String,
    pub sha: String,
    pub short_sha: String,
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

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>, // only set for renames
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
}
