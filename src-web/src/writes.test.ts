// Verifies: ADR-draft-merge-editor, ADR-0034, ADR-0036, G-16, Q-7
import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import type { Conflict, FileChange, Offered, Operation, Staging, Transfer, Transfers } from './types.ts';
import {
  OneWrite, actionLabel, branchRequest, operationName, commitRequest, confirmedWrite, stageToggle, stashRequest, tagRequest,
  type ResetMode, type Take, type Write, type WriteOf,
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
      path: 'f', oldPath: null, status: 'modified' as const, additions: 1, deletions: 0,
      staging: { staged, shown: { workdir: 'w', index: 'i' } },
    });
    expect(stageToggle(file('yes'))).toEqual({ kind: 'unstageFile', path: 'f', shown: 'i' });
    expect(stageToggle(file('partly'))).toEqual({ kind: 'stageFile', path: 'f', shown: 'w' });
    expect(stageToggle(file('no'))).toEqual({ kind: 'stageFile', path: 'f', shown: 'w' });
    const committed = { path: 'f', oldPath: null, status: 'modified' as const, additions: 1, deletions: 0 };
    expect(stageToggle(committed)).toBeNull();
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

describe('history', () => {
  test('taking a side goes ahead under that side', () => {
    const conflict = { path: 'f', ancestor: null, ours: 'o', theirs: 't' };
    expect(actionLabel({ kind: 'resolve', path: 'f', conflict, take: 'ours' })).toBe('Take ours');
    expect(actionLabel({ kind: 'resolve', path: 'f', conflict, take: 'theirs' })).toBe('Take theirs');
  });

  test('an operation reads as git names it, and so does the abort that discards it', () => {
    expect(operationName('cherryPick')).toBe('cherry-pick');
    expect(actionLabel({ kind: 'abort', operation: 'merge', head: 'h' })).toBe('Abort the merge');
    expect(actionLabel({ kind: 'reset', branch: 'm', tip: 't', commit: 'c', mode: 'hard' }))
      .toBe('Reset hard');
  });
});

describe('the JSON the core takes', () => {
  // The fixture is written by the core's test (crates/core/tests/write_fixture.rs), one example of
  // every write as the core serializes it; this side only reads it.
  const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'writes.json');

  /** The fields of every kind of `Write`; the type requires every kind and exactly its fields, so
      a kind or field changed in the union does not compile until it changes here too. */
  const fields: { [K in Write['kind']]: Record<Exclude<keyof WriteOf<K>, 'kind'>, true> } = {
    createTag: { name: true, commit: true, message: true },
    deleteTag: { name: true, commit: true, annotated: true },
    createStash: { message: true, untracked: true },
    applyStash: { index: true, commit: true, drop: true },
    dropStash: { index: true, commit: true },
    stageFile: { path: true, shown: true },
    unstageFile: { path: true, shown: true },
    commit: { message: true, head: true },
    createBranch: { name: true, commit: true, switch: true },
    switchBranch: { name: true, tip: true },
    detach: { commit: true },
    trackRemote: { remote: true, branch: true, tip: true },
    deleteBranch: { name: true, tip: true, merged: true },
    fetch: { remote: true },
    pull: { branch: true, tip: true },
    push: { branch: true, tip: true },
    merge: { branch: true, tip: true, commit: true, name: true },
    rebase: { branch: true, tip: true, onto: true },
    cherryPick: { branch: true, tip: true, commit: true },
    revert: { branch: true, tip: true, commit: true },
    reset: { branch: true, tip: true, commit: true, mode: true },
    resolve: { path: true, conflict: true, take: true },
    continue: { operation: true, head: true },
    abort: { operation: true, head: true },
  };
  const conflictFields: Record<keyof Conflict, true> = { path: true, ancestor: true, ours: true, theirs: true };
  const resetModes: Record<ResetMode, true> = { soft: true, mixed: true, hard: true };
  const takes: Record<Take, true> = { ours: true, theirs: true, asEdited: true };
  const operations: Record<Operation, true> = { merge: true, rebase: true, cherryPick: true, revert: true };

  const sorted = (keys: Iterable<string>) => [...keys].sort();

  test('every kind of the union is one the core writes, and no other', async () => {
    const writes = (await Bun.file(fixture).json()) as Write[];
    expect(writes.length).toBeGreaterThan(1);
    expect(sorted(writes.map((w) => w.kind))).toEqual(sorted(Object.keys(fields)));
  });

  test('every write carries the fields the union names it with', async () => {
    const writes = (await Bun.file(fixture).json()) as Write[];
    for (const write of writes) {
      const { kind, ...rest } = write;
      expect({ kind, fields: sorted(Object.keys(rest)) })
        .toEqual({ kind, fields: sorted(Object.keys(fields[kind] ?? {})) });
      if (write.kind === 'resolve') {
        expect(sorted(Object.keys(write.conflict))).toEqual(sorted(Object.keys(conflictFields)));
        expect(Object.keys(takes)).toContain(write.take);
      }
      if (write.kind === 'reset') expect(Object.keys(resetModes)).toContain(write.mode);
      if (write.kind === 'continue' || write.kind === 'abort') {
        expect(Object.keys(operations)).toContain(write.operation);
      }
    }
  });
});

describe('the JSON of a file the host sends', () => {
  // Written by the core's test (crates/core/tests/write_fixture.rs): a file of the
  // working-directory node, with its staging, and a file of a commit, without one.
  const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'files.json');

  /** The fields of `FileChange` and of its `Staging`; the types require exactly these. */
  const fileFields: Record<keyof FileChange, true> = {
    path: true, oldPath: true, status: true, additions: true, deletions: true, staging: true,
  };
  const stagingFields: Record<keyof Staging, true> = { staged: true, shown: true };

  test('a file carries the fields the interface reads, a staging only for the working directory', async () => {
    const [working, committed] = (await Bun.file(fixture).json()) as Record<string, unknown>[];
    expect(Object.keys(working).sort()).toEqual(Object.keys(fileFields).sort());
    expect(Object.keys(working.staging as object).sort()).toEqual(Object.keys(stagingFields).sort());
    expect(Object.keys((working.staging as Staging).shown).sort()).toEqual(['index', 'workdir']);
    expect('staging' in committed).toBe(false);
    expect(stageToggle(working as unknown as FileChange)).toEqual({ kind: 'stageFile', path: 'f', shown: 'w' });
    expect(stageToggle(committed as unknown as FileChange)).toBeNull();
  });
});

describe('the JSON of what the host offers', () => {
  // Written by the core's test (crates/core/tests/write_fixture.rs): the transfers of refs that
  // offer a fetch and a push and refuse a pull, and the operations of a commit beside a refusal,
  // as the host sends them (ADR-0036). This side only draws them, and reads their shape here.
  const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'offers.json');

  /** The fields of each answer; the types require exactly these. */
  const transferKeys: Record<keyof Transfers, true> = { fetch: true, pull: true, push: true };
  const offeredKeys: Record<keyof Extract<Transfer<'fetch'>, { write: unknown }>, true> = {
    write: true, canStop: true,
  };
  const reasonKeys: Record<keyof Extract<Offered, { reason: string }>, true> = { reason: true };
  const operationsKeys: Record<keyof Extract<Offered, { operations: unknown }>, true> = { operations: true };
  const sorted = (value: object) => Object.keys(value).sort();

  test('a transfer is a write with whether it stops, or a reason, and both arrive', async () => {
    const { transfers } = (await Bun.file(fixture).json()) as { transfers: Record<string, object> };
    expect(sorted(transfers)).toEqual(sorted(transferKeys));
    const shapes = Object.values(transfers).map(sorted);
    expect(shapes).toContainEqual(sorted(offeredKeys));
    expect(shapes).toContainEqual(sorted(reasonKeys));
    for (const shape of shapes) expect([sorted(offeredKeys), sorted(reasonKeys)]).toContainEqual(shape);
    const { fetch } = transfers as unknown as Transfers;
    expect('write' in fetch && fetch.write.kind === 'fetch' && typeof fetch.canStop).toBe('boolean');
  });

  test('a commit offers its operations as writes, or a reason', async () => {
    const { offered } = (await Bun.file(fixture).json()) as { offered: object[] };
    expect(offered.map(sorted)).toEqual([sorted(operationsKeys), sorted(reasonKeys)]);
    const [{ operations }] = offered as { operations: Write[] }[];
    expect(operations.map((w) => w.kind)).toEqual(['merge', 'rebase', 'cherryPick', 'revert', 'reset']);
  });
});
