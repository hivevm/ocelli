// Verifies: G-17
import { describe, expect, test } from 'bun:test';
import { ancestryRows, descendantRows, lineageRows } from './ancestry.ts';
import type { NodeJson } from './types.ts';

/** Minimal node fixture — only sha, row, parents, and children matter here. */
function node(sha: string, row: number, parents: string[], children: string[] = []): NodeJson {
  return {
    row,
    col: 0,
    kind: 'commit',
    sha,
    shortSha: sha.slice(0, 7),
    message: `commit ${sha}`,
    author: 'Test',
    authorDate: '',
    authorAvatar: '',
    committer: 'Test',
    date: '',
    committerAvatar: '',
    refs: [],
    parents,
    children,
  };
}

function graph(...nodes: NodeJson[]): Map<string, NodeJson> {
  return new Map(nodes.map((n) => [n.sha, n]));
}

describe('ancestryRows', () => {
  test('follows the parent chain and includes the start node', () => {
    // A(0) ─ B(1) ─ C(2); D(3) is unrelated.
    const bySha = graph(
      node('A', 0, ['B']),
      node('B', 1, ['C']),
      node('C', 2, []),
      node('D', 3, []),
    );

    expect(ancestryRows(bySha.get('A')!, bySha)).toEqual(new Set([0, 1, 2]));
  });

  test('a merge pulls in both parent lines', () => {
    // M(0) merges B(1) and F(2), both on C(3).
    const bySha = graph(
      node('M', 0, ['B', 'F']),
      node('B', 1, ['C']),
      node('F', 2, ['C']),
      node('C', 3, []),
    );

    expect(ancestryRows(bySha.get('M')!, bySha)).toEqual(new Set([0, 1, 2, 3]));
  });

  test('starting mid-graph excludes children and unrelated branches', () => {
    const bySha = graph(
      node('M', 0, ['B', 'F']),
      node('B', 1, ['C']),
      node('F', 2, ['C']),
      node('C', 3, []),
    );

    expect(ancestryRows(bySha.get('F')!, bySha)).toEqual(new Set([2, 3]));
  });

  test('parents outside the loaded set are skipped', () => {
    const bySha = graph(node('A', 0, ['missing']));

    expect(ancestryRows(bySha.get('A')!, bySha)).toEqual(new Set([0]));
  });

  test('shared ancestors are visited once', () => {
    // Diamond: M → (B, F) → C → root; must terminate and count C once.
    const bySha = graph(
      node('M', 0, ['B', 'F']),
      node('B', 1, ['C']),
      node('F', 2, ['C']),
      node('C', 3, ['R']),
      node('R', 4, []),
    );

    expect(ancestryRows(bySha.get('M')!, bySha).size).toBe(5);
  });
});

describe('descendantRows / lineageRows', () => {
  // M(0) merges B(1) and F(2), both on C(3); D(4) is an unrelated root.
  const bySha = graph(
    node('M', 0, ['B', 'F'], []),
    node('B', 1, ['C'], ['M']),
    node('F', 2, ['C'], ['M']),
    node('C', 3, [], ['B', 'F']),
    node('D', 4, [], []),
  );

  test('descendants follow child links up to the tips', () => {
    expect(descendantRows(bySha.get('C')!, bySha)).toEqual(new Set([0, 1, 2, 3]));
    expect(descendantRows(bySha.get('B')!, bySha)).toEqual(new Set([0, 1]));
  });

  test('a tip has no descendants beyond itself', () => {
    expect(descendantRows(bySha.get('M')!, bySha)).toEqual(new Set([0]));
  });

  test('lineage is the union of ancestors and descendants, excluding siblings', () => {
    // B's lineage: C below, M above — but not the sibling branch F, and not D.
    expect(lineageRows(bySha.get('B')!, bySha)).toEqual(new Set([0, 1, 3]));
  });
});
