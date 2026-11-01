import type { Write, WriteOf } from './writes.ts';

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
  /** A stash node's place `n` in the stash, `stash@{n}`, which a stash write names. */
  stash?: number;
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
  /** The lane a merge out of the main line runs up between its ends (ADR-0015); absent otherwise. */
  track?: number;
}

/** Rows `top..top + nodes.length` of the history the host keeps, with every edge crossing them
 *  (ADR-0019). With a commit selected, `litNodes` and `litEdges` say which of them belong to its
 *  lineage; null when nothing is selected. */
export interface RowWindow {
  /** The load the rows belong to; an answer from an older load is dropped. */
  generation: number;
  top: number;
  /** The lanes the window's nodes and edges reach. */
  lanes: number;
  nodes: NodeJson[];
  edges: Edge[];
  /** Each edge's corners from its parent to its child, as `[row, lane]` (ADR-0015). */
  paths: [number, number][][];
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
  /** The row to select now: the commit a write left selected, laid out by this growth. */
  select: number | null;
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
  /** The operations of G-13 the commit offers on the checked-out branch, as the core decides them
      (ADR-0031, ADR-0022); null for a stash or the working directory, and until the load's refs
      are read. */
  operations: Offered | null;
}

/** The operations a commit offers, each with its write, or why there are none. */
export type Offered = { operations: Write[] } | { reason: string };

/** A transfer the refs make possible, with whether it can be stopped while it runs, or why there
    is none, as the core decides (ADR-0034). */
export type Transfer<K extends 'fetch' | 'pull' | 'push'> =
  | { write: WriteOf<K>; canStop: boolean }
  | { reason: string };

/** The transfers the toolbar offers. */
export interface Transfers {
  fetch: Transfer<'fetch'>;
  pull: Transfer<'pull'>;
  push: Transfer<'push'>;
}

/** The `load-refs` payload: the refs, and the transfers the core decides from them (ADR-0022). */
export interface LoadedRefs {
  refs: RepoRefs;
  transfers: Transfers;
}

/** Every row matching a search, newest first, and the nodes of the first ones. */
export interface Found {
  generation: number;
  rows: number[];
  hits: NodeJson[];
}

/** A folder below the remote server's root, marked where it holds a `.git` (ADR-0037). */
export interface Folder {
  name: string;
  /** Relative to the server root, as `open_repo` and `list_folders` take it. */
  path: string;
  repository: boolean;
}

/** The remote server's `list_folders` answer: its counterpart of the desktop's folder dialog. */
export interface Listing {
  /** The folder listed, relative to the server root, `.` for the root itself. */
  path: string;
  repository: boolean;
  folders: Folder[];
  /** Whether the folder holds more than the server lists. */
  more: boolean;
}

// ── Repository sidebar (ADR-0027) ──────────────────────────────────────────
// Refs and checkout locations, not history: each entry only carries the commit
// it resolves to, which is all the sidebar needs to navigate there.

export interface LocalBranch {
  name: string;
  sha: string;
  shortSha: string;
  isHead: boolean;
  /** Full upstream name (e.g. `origin/main`), null when none is configured. */
  upstream: string | null;
  /** Position against the upstream as of the last fetch. */
  ahead: number;
  behind: number;
  /** Whether every commit of the branch is in its upstream, or in HEAD without one. */
  merged: boolean;
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

/** What kind of repository the current one sits inside (ADR-0028). */
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
  /** A tag object with a message of its own, rather than a bare reference. */
  annotated: boolean;
}

export interface RepoRefs {
  locals: LocalBranch[];
  remotes: Remote[];
  worktrees: Worktree[];
  tags: Tag[];
  submodules: Submodule[];
  /** Null when this repository is neither a submodule nor a linked worktree. */
  parent: ParentRepo | null;
  /** The default remote, null where there is none to choose. */
  defaultRemote: string | null;
  /** The operation stopped in the repository, whoever started it, null where none is. */
  inProgress: InProgress | null;
}

/** An operation that can stop on a conflict. */
export type Operation = 'merge' | 'rebase' | 'cherryPick' | 'revert';

/** A conflicted file and the blob ids of its sides, null where a side has no file. */
export interface Conflict {
  path: string;
  ancestor: string | null;
  ours: string | null;
  theirs: string | null;
}

/** A block of a conflicted file: the lines of its first and last marker, counted from 1, and
    the text of each side, each line with its line ending. The base is the file's own `|||||||`
    section where `baseInFile`, else one found in memory that none of the file's lines hold. */
export interface MergeBlock {
  firstLine: number;
  lastLine: number;
  ours: string;
  theirs: string;
  base: string | null;
  baseInFile: boolean;
}

/** A conflicted file the merge editor opens, as the working directory holds it (ADR-0031). */
export interface MergeFile {
  text: string;
  /** The blob id of the file it was read from, which saving expects to find again. */
  blob: string;
  blocks: MergeBlock[];
}

/** A merge, rebase, cherry-pick, or revert stopped in the repository. */
export interface InProgress {
  operation: Operation;
  conflicts: Conflict[];
  /** The conflicted files whose sides both hold text, which the merge editor opens. */
  editable: string[];
  /** The commit HEAD is at, which continuing and aborting expect. */
  head: string;
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
  /** For the working-directory node's files only, each of which carries one: how the file
      stands in the index. */
  staging?: Staging;
}

/** How a file of the working-directory node stands in the index: how much of its change is
    staged, and the content shown, by blob id, which staging and unstaging expect to find. */
export interface Staging {
  staged: 'no' | 'yes' | 'partly';
  shown: { workdir: string | null; index: string | null };
}

/** The remote server's own event, `repo-written`: another tab wrote to the repository this tab
    has open, which it loads again; the payload is what the status bar says. The desktop host
    never sends it (ADR-0035). */
export type RepoWritten = string;
