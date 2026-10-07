// Verifies: ADR-0033, ADR-0034, G-7, Q-6
import { describe, expect, test } from 'bun:test';
import {
  Reselect, actionLabel, confirmedWrite, stashIndex, stashRequest, tagRequest, type Write,
} from './writes.ts';

describe('tagRequest', () => {
  test('a message makes an annotated tag, a blank one a lightweight tag', () => {
    expect(tagRequest('abc', ' v1 ', 'The first release')).toEqual({
      kind: 'createTag', name: 'v1', commit: 'abc', message: 'The first release',
    });
    expect(tagRequest('abc', 'mark', '  \n')).toEqual({
      kind: 'createTag', name: 'mark', commit: 'abc', message: null,
    });
  });

  test('a tag without a name is no tag', () => {
    expect(tagRequest('abc', '  ', 'message')).toBeNull();
  });
});

describe('Reselect', () => {
  test('looks the commit up only in loads newer than the write', () => {
    const reselect = new Reselect();
    reselect.remember('c', 3);
    expect(reselect.wanted({ generation: 3, loading: true })).toBeNull();
    expect(reselect.wanted({ generation: 4, loading: true })).toBe('c');
  });

  test('selects the commit once laid out, and only once', () => {
    const reselect = new Reselect();
    reselect.remember('c', 1);
    const growing = { generation: 2, loading: true };
    expect(reselect.settle('c', false, growing)).toBe(false);
    expect(reselect.wanted(growing)).toBe('c');
    expect(reselect.settle('c', true, growing)).toBe(true);
    expect(reselect.settle('c', true, growing)).toBe(false);
    expect(reselect.wanted(growing)).toBeNull();
  });

  test('gives the commit up when the reload ends without it or another is selected', () => {
    const reselect = new Reselect();
    reselect.remember('c', 1);
    expect(reselect.settle('c', false, { generation: 2, loading: false })).toBe(false);
    expect(reselect.wanted({ generation: 2, loading: false })).toBeNull();
    reselect.remember('d', 2);
    reselect.forget();
    expect(reselect.wanted({ generation: 3, loading: true })).toBeNull();
  });
});

describe('stashes', () => {
  test('a blank message is none, the untracked switch as set', () => {
    expect(stashRequest(' ', true)).toEqual({ kind: 'createStash', message: null, untracked: true });
    expect(stashRequest('wip', false)).toEqual({ kind: 'createStash', message: 'wip', untracked: false });
  });

  test("a stash node's place is read from its ref", () => {
    expect(stashIndex(['📦 stash@{2}'])).toBe(2);
    expect(stashIndex(['main', '🏷 v1'])).toBeNull();
  });
});

describe('confirmedWrite', () => {
  const drop: Write = { kind: 'dropStash', index: 0, commit: 's' };
  const tag: Write = { kind: 'createTag', name: 'v1', commit: 'c', message: null };
  const discards = async (w: Write) => (w.kind === 'dropStash' ? 'the stash stash@{0}' : null);

  test('asks in the core\'s words, and writes confirmed only on a yes', async () => {
    const asked: string[] = [];
    const calls: [Write, boolean][] = [];
    const call = async (w: Write, confirmed: boolean) => { calls.push([w, confirmed]); };
    const no = await confirmedWrite(drop, discards, async (lost) => { asked.push(lost); return false; }, call);
    expect(no).toBe('kept');
    expect(calls).toEqual([]);
    expect(await confirmedWrite(drop, discards, async () => true, call)).toBe('written');
    expect(calls).toEqual([[drop, true]]);
    expect(asked).toEqual(['the stash stash@{0}']);
    expect(actionLabel(drop)).toBe('Drop stash');
  });

  test('a write that discards nothing asks nothing and goes unconfirmed', async () => {
    const calls: [Write, boolean][] = [];
    const never = async () => { throw new Error('asked'); };
    await confirmedWrite(tag, discards, never, async (w, c) => { calls.push([w, c]); });
    expect(calls).toEqual([[tag, false]]);
  });
});
