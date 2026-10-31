// Verifies: ADR-0041, ADR-0028
import { describe, expect, test } from 'bun:test';
import { anyLeaf, countLeaves, filterByName, groupByFolder, refShas } from './refTree.ts';

interface Ref {
  name: string;
}

function refs(...names: string[]): Ref[] {
  return names.map(name => ({ name }));
}

describe('groupByFolder', () => {
  test('keeps names without a slash flat in the root', () => {
    const root = groupByFolder(refs('main', 'develop'));
    expect(root.folders).toHaveLength(0);
    expect(root.leaves.map(l => l.label)).toEqual(['main', 'develop']);
  });

  test('groups names that share a prefix into one folder', () => {
    const root = groupByFolder(refs('feature/sidebar', 'feature/graph', 'main'));
    expect(root.leaves.map(l => l.label)).toEqual(['main']);
    expect(root.folders).toHaveLength(1);

    const feature = root.folders[0];
    expect(feature.name).toBe('feature');
    expect(feature.path).toBe('feature');
    expect(feature.leaves.map(l => l.label)).toEqual(['sidebar', 'graph']);
  });

  test('nests deeper paths and gives every folder its full path', () => {
    const root = groupByFolder(refs('release/2026/08/hotfix'));
    const release = root.folders[0];
    const year = release.folders[0];
    const month = year.folders[0];

    expect([release.path, year.path, month.path]).toEqual([
      'release',
      'release/2026',
      'release/2026/08',
    ]);
    expect(month.leaves[0].label).toBe('hotfix');
    // The entry keeps its full name, which is what navigation and filtering use.
    expect(month.leaves[0].item.name).toBe('release/2026/08/hotfix');
  });

  test('a folder and a branch of the same name coexist', () => {
    const root = groupByFolder(refs('feature', 'feature/sidebar'));
    expect(root.leaves.map(l => l.label)).toEqual(['feature']);
    expect(root.folders[0].leaves.map(l => l.label)).toEqual(['sidebar']);
  });

  test('empty segments never produce nameless folders', () => {
    const root = groupByFolder(refs('/leading', 'trailing/', 'double//slash', ''));
    const names = (f: ReturnType<typeof groupByFolder<Ref>>): string[] => [
      ...f.folders.map(sub => sub.name),
      ...f.folders.flatMap(names),
    ];
    expect(names(root)).not.toContain('');
    expect(countLeaves(root)).toBe(3);
  });
});

describe('filterByName', () => {
  const all = refs('main', 'feature/Sidebar', 'release/2.1');

  test('an empty or whitespace-only query keeps everything', () => {
    expect(filterByName(all, '')).toHaveLength(3);
    expect(filterByName(all, '   ')).toHaveLength(3);
  });

  test('matches case-insensitively anywhere in the name', () => {
    expect(filterByName(all, 'SIDE').map(r => r.name)).toEqual(['feature/Sidebar']);
    expect(filterByName(all, 'ai').map(r => r.name)).toEqual(['main']);
  });

  test('matches on the folder part too, so a whole folder stays findable', () => {
    expect(filterByName(all, 'feature/').map(r => r.name)).toEqual(['feature/Sidebar']);
  });

  test('a query that matches nothing yields nothing', () => {
    expect(filterByName(all, 'nope')).toEqual([]);
  });
});

describe('countLeaves', () => {
  test('counts entries across every level', () => {
    expect(countLeaves(groupByFolder(refs('a', 'b/c', 'b/d/e')))).toBe(3);
  });
});

describe('anyLeaf', () => {
  const tree = groupByFolder(refs('main', 'feat/a', 'feat/deep/b'));

  test('finds an entry at any depth below the folder', () => {
    expect(anyLeaf(tree, r => r.name === 'main')).toBe(true);
    expect(anyLeaf(tree, r => r.name === 'feat/deep/b')).toBe(true);
  });

  test('a folder only reports what is actually inside it', () => {
    const feat = tree.folders.find(f => f.name === 'feat')!;
    expect(anyLeaf(feat, r => r.name === 'feat/deep/b')).toBe(true);
    expect(anyLeaf(feat, r => r.name === 'main')).toBe(false);
  });

  test('no match anywhere is false', () => {
    expect(anyLeaf(tree, r => r.name === 'nope')).toBe(false);
  });
});

describe('refShas', () => {
  test('lists each commit a branch, remote branch, working tree, or tag points at, once', () => {
    const shas = refShas({
      locals: [{ sha: 'a' }, { sha: 'b' }],
      remotes: [{ branches: [{ sha: 'a' }, { sha: 'c' }] }],
      worktrees: [{ sha: 'd' }, { sha: '' }],
      tags: [{ sha: 'e' }],
    });
    expect(shas).toEqual(['a', 'b', 'c', 'd', 'e']);
  });
});
