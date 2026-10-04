// Verifies: ADR-0020
import { describe, expect, test } from 'bun:test';
import { buildFileTree, type TreeDirNode, type TreeNode } from './fileTree.ts';
import type { FileChange } from './types.ts';

function change(path: string): FileChange {
  return { path, oldPath: null, status: 'modified', additions: 1, deletions: 0 };
}

/** Depth-first list of "kind:name" labels, so a tree's shape is easy to assert on. */
function shape(nodes: TreeNode[], depth = 0): string[] {
  return nodes.flatMap((n) =>
    n.type === 'dir'
      ? [`${'  '.repeat(depth)}dir:${n.name}(${n.fileCount})`, ...shape(n.children, depth + 1)]
      : [`${'  '.repeat(depth)}file:${n.name}`],
  );
}

const dirs = (nodes: TreeNode[]): TreeDirNode[] =>
  nodes.filter((n): n is TreeDirNode => n.type === 'dir');

describe('buildFileTree', () => {
  test('a flat changeset produces files at the root', () => {
    expect(shape(buildFileTree([change('README.md'), change('LICENSE')]))).toEqual([
      'file:LICENSE',
      'file:README.md',
    ]);
  });

  test('single-child folder chains are collapsed into one row', () => {
    // ADR-0020: "src/web" instead of "src" > "web".
    const tree = buildFileTree([change('src/web/main.ts')]);
    expect(dirs(tree).map((d) => d.name)).toEqual(['src/web']);
    expect(shape(tree)).toEqual(['dir:src/web(1)', '  file:main.ts']);
  });

  test('a chain stops collapsing where it branches', () => {
    const tree = buildFileTree([change('a/b/c/one.ts'), change('a/b/d/two.ts')]);
    // a/b has two children, so it collapses only down to that point.
    expect(dirs(tree).map((d) => d.name)).toEqual(['a/b']);
    expect(dirs(dirs(tree)[0].children).map((d) => d.name)).toEqual(['c', 'd']);
  });

  test('a folder holding both a file and a subfolder does not collapse', () => {
    const tree = buildFileTree([change('src/main.ts'), change('src/web/app.ts')]);
    expect(dirs(tree).map((d) => d.name)).toEqual(['src']);
    expect(shape(tree)).toEqual([
      'dir:src(2)',
      '  dir:web(1)',
      '    file:app.ts',
      '  file:main.ts',
    ]);
  });

  test('folder counts sum recursively', () => {
    const tree = buildFileTree([
      change('src/a.ts'),
      change('src/deep/b.ts'),
      change('src/deep/nested/c.ts'),
      change('other/d.ts'),
    ]);
    const byName = Object.fromEntries(dirs(tree).map((d) => [d.name, d.fileCount]));
    expect(byName).toEqual({ src: 3, other: 1 });
  });

  test('folders sort before files, each alphabetically', () => {
    const tree = buildFileTree([change('zeta.ts'), change('alpha.ts'), change('zdir/x.ts')]);
    expect(shape(tree)).toEqual(['dir:zdir(1)', '  file:x.ts', 'file:alpha.ts', 'file:zeta.ts']);
  });

  test('an empty changeset yields an empty tree', () => {
    expect(buildFileTree([])).toEqual([]);
  });

  test('every file node keeps its full path and original change', () => {
    const c = change('src/web/main.ts');
    const tree = buildFileTree([c]);
    const dir = dirs(tree)[0];
    const file = dir.children[0];
    expect(file.type).toBe('file');
    if (file.type === 'file') {
      expect(file.path).toBe('src/web/main.ts');
      expect(file.file).toBe(c);
    }
  });
});
