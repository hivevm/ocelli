// Verifies: ADR-0033, ADR-0034, G-7, Q-6
import { describe, expect, test } from 'bun:test';
import {
  OneWrite, actionLabel, branchRequest, commitRequest, confirmedWrite, stageToggle, stashRequest, tagRequest,
  type Write,
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

describe('stashes', () => {
  test('a blank message is none, the untracked switch as set', () => {
    expect(stashRequest(' ', true)).toEqual({ kind: 'createStash', message: null, untracked: true });
    expect(stashRequest('wip', false)).toEqual({ kind: 'createStash', message: 'wip', untracked: false });
  });
});

describe('confirmedWrite', () => {
  const drop: Write = { kind: 'dropStash', index: 0, commit: 's' };
  const tag: Write = { kind: 'createTag', name: 'v1', commit: 'c', message: null };
  const discards = async (w: Write) => (w.kind === 'dropStash' ? 'the stash stash@{0}' : null);

  test('asks in the core\'s words, and writes confirmed only on a yes', async () => {
    const asked: string[] = [];
    const calls: [Write, boolean][] = [];
    const call = async (w: Write, confirmed: boolean) => { calls.push([w, confirmed]); return null; };
    const no = await confirmedWrite(drop, discards, async (lost) => { asked.push(lost); return false; }, call);
    expect(no).toEqual({ kind: 'kept' });
    expect(calls).toEqual([]);
    expect(await confirmedWrite(drop, discards, async () => true, call))
      .toEqual({ kind: 'written', made: null });
    expect(calls).toEqual([[drop, true]]);
    expect(asked).toEqual(['the stash stash@{0}']);
    expect(actionLabel(drop)).toBe('Drop stash');
  });

  test('a write that discards nothing asks nothing and goes unconfirmed', async () => {
    const calls: [Write, boolean][] = [];
    const never = async () => { throw new Error('asked'); };
    const made = await confirmedWrite(tag, discards, never, async (w, c) => {
      calls.push([w, c]);
      return 'new';
    });
    expect(calls).toEqual([[tag, false]]);
    expect(made).toEqual({ kind: 'written', made: 'new' });
  });
});

describe('OneWrite', () => {
  /** A write whose answer the test hands over when it likes. */
  function pending(): { write: () => Promise<string | null>; answer: (e: string | null) => void } {
    let answer!: (e: string | null) => void;
    const promise = new Promise<string | null>((resolve) => { answer = resolve; });
    return { write: () => promise, answer };
  }

  test('starts no second write while one runs, from whatever control', async () => {
    const one = new OneWrite();
    const first = pending();
    const running = one.run('c', first.write, () => 'c');
    expect(one.running).toBe(true);
    let started = false;
    const second = await one.run('c', async () => { started = true; return null; }, () => 'c');
    expect(second).toBeNull();
    expect(started).toBe(false);
    first.answer('refused');
    expect(await running).toEqual({ error: 'refused' });
    expect(one.running).toBe(false);
  });

  test('drops the answer once another commit is selected', async () => {
    const one = new OneWrite();
    const first = pending();
    let selected = 'c';
    const running = one.run('c', first.write, () => selected);
    selected = 'd';
    first.answer('refused');
    expect(await running).toBeNull();
  });
});

describe('staging and committing', () => {
  test('the toggle unstages a staged file and stages the rest whole, as shown', () => {
    const file = (staged: 'no' | 'yes' | 'partly') => ({
      path: 'f', oldPath: null, status: 'modified' as const, additions: 1, deletions: 0, staged,
      shown: { workdir: 'w', index: 'i' },
    });
    expect(stageToggle(file('yes'))).toEqual({ kind: 'unstageFile', path: 'f', shown: 'i' });
    expect(stageToggle(file('partly'))).toEqual({ kind: 'stageFile', path: 'f', shown: 'w' });
    expect(stageToggle(file('no'))).toEqual({ kind: 'stageFile', path: 'f', shown: 'w' });
  });

  test('a commit needs a message', () => {
    expect(commitRequest('h', ' \n')).toBeNull();
    expect(commitRequest('h', 'Change f')).toEqual({ kind: 'commit', message: 'Change f', head: 'h' });
  });
});

describe('branches', () => {
  test('a branch needs a name, and is switched to as the form says', () => {
    expect(branchRequest('c', '  ', true)).toBeNull();
    expect(branchRequest('c', ' topic ', false))
      .toEqual({ kind: 'createBranch', name: 'topic', commit: 'c', switch: false });
  });

  test('deleting a branch goes ahead under its own label', () => {
    expect(actionLabel({ kind: 'deleteBranch', name: 'x', tip: 't', merged: false }))
      .toBe('Delete branch');
  });
});
