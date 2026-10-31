// Verifies: ADR-0031, ADR-0033, G-12, G-16
import { describe, expect, test } from 'bun:test';
import { commitChips } from './refChips.ts';
import type { LocalBranch, RepoRefs } from './types.ts';

type Refs = Pick<RepoRefs, 'locals' | 'remotes' | 'tags'>;

function local(name: string, sha: string, isHead = false, merged = true): LocalBranch {
  return { name, sha, shortSha: sha, isHead, upstream: null, ahead: 0, behind: 0, merged };
}

function remote(name: string, branches: [string, string][]) {
  return { name, branches: branches.map(([b, sha]) => ({ name: b, sha, shortSha: sha })) };
}

describe('commitChips', () => {
  test('a local branch offers switch and delete, a remote one switch, a tag delete, and the checked-out branch nothing', () => {
    const refs: Refs = {
      locals: [local('main', 'c', true), local('topic', 'c', false, false), local('other', 'd')],
      remotes: [remote('origin', [['main', 'c'], ['other', 'd']])],
      tags: [{ name: 'v1', sha: 'c', shortSha: 'c', annotated: true }],
    };
    const chips = commitChips(['main', 'topic', 'origin/main', '🏷 v1'], refs, 'c');
    expect(chips.map(c => [c.ref, c.kind, c.offers.map(o => [o.action, o.write])])).toEqual([
      ['main', 'local', []],
      ['topic', 'local', [
        ['switch', { kind: 'switchBranch', name: 'topic', tip: 'c' }],
        ['delete', { kind: 'deleteBranch', name: 'topic', tip: 'c', merged: false }],
      ]],
      ['origin/main', 'remote', [
        ['switch', { kind: 'trackRemote', remote: 'origin', branch: 'main', tip: 'c' }],
      ]],
      ['🏷 v1', 'tag', [
        ['delete', { kind: 'deleteTag', name: 'v1', commit: 'c', annotated: true }],
      ]],
    ]);
    expect(chips[1].offers.map(o => o.label)).toEqual(['Switch to topic', 'Delete the branch topic']);
    expect(chips[2].offers[0].label).toBe('Switch to a local branch tracking origin/main');
    expect(chips[3].offers[0].label).toBe('Delete the tag v1');
  });

  test('remotes a and a/b each keep their own branch, though their chips read alike', () => {
    const refs: Refs = {
      locals: [],
      remotes: [remote('a', [['b/c', 'c'], ['x', 'c']]), remote('a/b', [['c', 'c'], ['x', 'c']])],
      tags: [],
    };
    const tracked = (chips: string[]) => commitChips(chips, refs, 'c')
      .map(c => c.offers.map(o => o.write));
    expect(tracked(['a/b/c', 'a/b/c'])).toEqual([
      [{ kind: 'trackRemote', remote: 'a', branch: 'b/c', tip: 'c' }],
      [{ kind: 'trackRemote', remote: 'a/b', branch: 'c', tip: 'c' }],
    ]);
    expect(tracked(['a/x', 'a/b/x'])).toEqual([
      [{ kind: 'trackRemote', remote: 'a', branch: 'x', tip: 'c' }],
      [{ kind: 'trackRemote', remote: 'a/b', branch: 'x', tip: 'c' }],
    ]);
  });

  test('a local branch and a remote one of the same text are two chips, each its own', () => {
    const refs: Refs = {
      locals: [local('origin/main', 'c')],
      remotes: [remote('origin', [['main', 'c']])],
      tags: [],
    };
    const writes = commitChips(['origin/main', 'origin/main'], refs, 'c')
      .map(c => c.offers.map(o => o.write.kind));
    expect(writes).toEqual([['switchBranch', 'deleteBranch'], ['trackRemote']]);
  });

  test('a ref on another commit, or refs not yet read, offer nothing', () => {
    const refs: Refs = {
      locals: [local('topic', 'd')],
      remotes: [remote('origin', [['topic', 'd']])],
      tags: [{ name: 'v1', sha: 'd', shortSha: 'd', annotated: false }],
    };
    const chips = ['topic', 'origin/topic', '🏷 v1'];
    expect(commitChips(chips, refs, 'c').map(c => c.offers)).toEqual([[], [], []]);
    expect(commitChips(chips, refs, 'd').map(c => c.offers.length)).toEqual([2, 1, 1]);
    expect(commitChips(chips, null, 'd').map(c => [c.ref, c.kind, c.offers]))
      .toEqual([['topic', 'local', []], ['origin/topic', 'remote', []], ['🏷 v1', 'tag', []]]);
  });
});
