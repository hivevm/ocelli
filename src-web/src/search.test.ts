// Verifies: G-16
import { describe, expect, test } from 'bun:test';
import { searchNodes } from './search.ts';
import type { NodeJson } from './types.ts';

function node(sha: string, row: number, extra: Partial<NodeJson> = {}): NodeJson {
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
    parents: [],
    children: [],
    ...extra,
  };
}

const shas = (nodes: NodeJson[]): string[] => nodes.map((n) => n.sha);

describe('searchNodes', () => {
  const nodes = [
    node('c0ffee1', 2, { message: 'fix: align the header', author: 'Alice' }),
    node('deadbee', 0, { message: 'feat: add search', author: 'Bob', refs: ['feature/search'] }),
    node('abc1234', 1, { message: 'docs: readme', author: 'alice', committer: 'Carol' }),
  ];

  test('a blank query matches nothing', () => {
    expect(searchNodes(nodes, '')).toEqual([]);
    expect(searchNodes(nodes, '   ')).toEqual([]);
  });

  test('matches the message case-insensitively', () => {
    expect(shas(searchNodes(nodes, 'HEADER'))).toEqual(['c0ffee1']);
  });

  test('matches author and committer', () => {
    expect(shas(searchNodes(nodes, 'alice'))).toEqual(['abc1234', 'c0ffee1']);
    expect(shas(searchNodes(nodes, 'carol'))).toEqual(['abc1234']);
  });

  test('matches the sha', () => {
    expect(shas(searchNodes(nodes, 'DEADBE'))).toEqual(['deadbee']);
  });

  test('matches ref names', () => {
    expect(shas(searchNodes(nodes, 'feature/'))).toEqual(['deadbee']);
  });

  test('results are ordered by row, newest first', () => {
    expect(shas(searchNodes(nodes, ':'))).toEqual(['deadbee', 'abc1234', 'c0ffee1']);
  });
});
