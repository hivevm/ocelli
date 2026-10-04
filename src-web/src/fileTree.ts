import type { FileChange } from './types.ts';

export interface TreeFileNode {
  type: 'file';
  name: string;
  path: string;
  file: FileChange;
}

export interface TreeDirNode {
  type: 'dir';
  name: string;
  path: string;
  children: TreeNode[];
  fileCount: number;
}

export type TreeNode = TreeDirNode | TreeFileNode;

interface DirBuilder {
  name: string;
  path: string;
  dirs: Map<string, DirBuilder>;
  files: TreeFileNode[];
}

function newDir(name: string, path: string): DirBuilder {
  return { name, path, dirs: new Map(), files: [] };
}

/** Build a directory tree from a flat list of file changes. */
export function buildFileTree(files: FileChange[]): TreeNode[] {
  const root = newDir('', '');

  for (const file of files) {
    const parts = file.path.split('/');
    const fileName = parts.pop()!;

    let dir = root;
    let acc = '';
    for (const part of parts) {
      acc = acc ? `${acc}/${part}` : part;
      let child = dir.dirs.get(part);
      if (!child) {
        child = newDir(part, acc);
        dir.dirs.set(part, child);
      }
      dir = child;
    }
    dir.files.push({ type: 'file', name: fileName, path: file.path, file });
  }

  // The root is a synthetic container, not a directory: it must not take part in
  // chain collapsing, or its name would be folded away and the top level lost.
  return childrenOf(root);
}

/** Folders first, then files, each alphabetically. */
function childrenOf(dir: DirBuilder): TreeNode[] {
  const childDirs = [...dir.dirs.values()].map(finalize);
  childDirs.sort((a, b) => a.name.localeCompare(b.name));
  const childFiles = [...dir.files].sort((a, b) => a.name.localeCompare(b.name));
  return [...childDirs, ...childFiles];
}

/** Recursively convert builders to nodes, collapsing single-folder chains. */
function finalize(dir: DirBuilder): TreeDirNode {
  // Collapse chains: a dir that holds exactly one subdir and no files
  // becomes "a/b" instead of nesting "a" > "b".
  let name = dir.name;
  let path = dir.path;
  let cur = dir;
  while (cur.files.length === 0 && cur.dirs.size === 1) {
    const only = cur.dirs.values().next().value as DirBuilder;
    name = `${name}/${only.name}`;
    path = only.path;
    cur = only;
  }

  const children = childrenOf(cur);
  const fileCount = children.reduce(
    (sum, child) => sum + (child.type === 'dir' ? child.fileCount : 1),
    0,
  );

  return { type: 'dir', name, path, children, fileCount };
}
