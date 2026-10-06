export type CommitKind = 'commit' | 'stash' | 'working';

export interface NodeJson {
  row: number;
  col: number;
  kind: CommitKind;
  sha: string;
  shortSha: string;
  message: string;
  author: string;
  authorDate: string;
  authorAvatar: string;
  committer: string;
  date: string;
  committerAvatar: string;
  refs: string[];
  /** Parent SHAs in commit order (first parent first); may include SHAs outside the loaded set. */
  parents: string[];
  /** SHAs of loaded children, newest (lowest row) first. */
  children: string[];
}

export interface Edge {
  sourceRow: number;
  sourceCol: number;
  targetRow: number;
  targetCol: number;
  isBranch: boolean;
  minRow: number;
  maxRow: number;
}

/** Rows `top..top + nodes.length` of the history the host keeps, with every edge crossing them
 *  (ADR-0031). With a commit selected, `litNodes` and `litEdges` say which of them belong to its
 *  lineage; null when nothing is selected. */
export interface RowWindow {
  /** The load the rows belong to; an answer from an older load is dropped. */
  generation: number;
  top: number;
  /** The lanes the window's nodes and edges reach. */
  lanes: number;
  nodes: NodeJson[];
  edges: Edge[];
  litNodes: boolean[] | null;
  litEdges: boolean[] | null;
}

/** How far the history of a load has come; a new `generation` is a new load. */
export interface Growth {
  generation: number;
  rows: number;
  lanes: number;
  loading: boolean;
  status: string;
}

/** A parent or child of the selected commit; `row` is null until it is laid out. */
export interface Relative {
  sha: string;
  label: string;
  row: number | null;
}

export interface Selected {
  generation: number;
  node: NodeJson;
  relatives: Relative[];
}

/** Every row matching a search, newest first, and the nodes of the first ones. */
export interface Found {
  generation: number;
  rows: number[];
  hits: NodeJson[];
}

// ── Repository sidebar (ADR-0026) ──────────────────────────────────────────
// Refs and checkout locations, not history: each entry only carries the commit
// it resolves to, which is all the sidebar needs to navigate there.

export interface LocalBranch {
  name: string;
  sha: string;
  shortSha: string;
  isHead: boolean;
  /** Full upstream name (e.g. `origin/main`), null when none is configured. */
  upstream: string | null;
  /** Position against the upstream as of the last fetch — the viewer never fetches. */
  ahead: number;
  behind: number;
}

export interface RemoteBranch {
  /** Name without the remote prefix — the remote it sits under supplies that. */
  name: string;
  sha: string;
  shortSha: string;
}

export interface Remote {
  name: string;
  branches: RemoteBranch[];
}

export interface Worktree {
  name: string;
  path: string;
  /** Checked-out branch, null for a detached HEAD. */
  branch: string | null;
  sha: string;
  shortSha: string;
  /** The repository's main working tree, which git2 does not list as a worktree. */
  isMain: boolean;
  /** The working tree currently being viewed — selecting it would go nowhere. */
  isCurrent: boolean;
}

/** Serde serializes the Rust enum variants camelCase. */
export type SubmoduleState = 'uninitialized' | 'modified' | 'inSync';

export interface Submodule {
  name: string;
  /** Path relative to the superproject's working directory. */
  path: string;
  /** Absolute path of the working copy; empty when uninitialized. */
  workdir: string;
  url: string | null;
  /** The commit the superproject pins — a commit of the *submodule's* history. */
  sha: string;
  shortSha: string;
  /** The commit actually checked out, null when there is no working copy. */
  checkedOutShortSha: string | null;
  state: SubmoduleState;
}

/** What kind of repository the current one sits inside (ADR-0027). */
export type ParentKind = 'superproject' | 'mainWorktree';

export interface ParentRepo {
  kind: ParentKind;
  name: string;
  path: string;
}

export interface Tag {
  name: string;
  sha: string;
  shortSha: string;
}

export interface RepoRefs {
  locals: LocalBranch[];
  remotes: Remote[];
  worktrees: Worktree[];
  tags: Tag[];
  submodules: Submodule[];
  /** Null when this repository is neither a submodule nor a linked worktree. */
  parent: ParentRepo | null;
}

/** Serde serializes the Rust enum variants camelCase — the statuses arrive lowercase. */
export type FileStatus = 'added' | 'modified' | 'deleted' | 'renamed' | 'copied' | 'unknown';

/** Serde serializes the Rust enum variants camelCase — the kinds arrive lowercase. */
export type DiffLineKind = 'add' | 'delete' | 'context' | 'hunk' | 'meta';

export interface DiffLine {
  kind: DiffLineKind;
  content: string;
  oldLineno: number | null;
  newLineno: number | null;
}

export interface FileChange {
  path: string;
  oldPath: string | null;
  status: FileStatus;
  additions: number;
  deletions: number;
}
