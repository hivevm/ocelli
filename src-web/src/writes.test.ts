// Verifies: ADR-0033, ADR-0034, G-7, Q-6
import { describe, expect, test } from 'bun:test';
import { Reselect, deleteTag, deletionLoss, tagRequest, type ShownTag } from './writes.ts';

describe('tagRequest', () => {
  test('a message makes an annotated tag, a blank one a lightweight tag', () => {
    expect(tagRequest('abc', ' v1 ', 'The first release')).toEqual({
      sha: 'abc', name: 'v1', message: 'The first release',
    });
    expect(tagRequest('abc', 'mark', '  \n')).toEqual({ sha: 'abc', name: 'mark', message: null });
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

describe('deleteTag', () => {
  const v1: ShownTag = { name: 'v1', sha: 'abc', annotated: true };

  test('names what is lost, the message of an annotated tag with it', () => {
    expect(deletionLoss(v1)).toBe('the tag v1 and its message');
    expect(deletionLoss({ ...v1, annotated: false })).toBe('the tag v1');
  });

  test('calls the host, confirmed, only once the developer says yes', async () => {
    const asked: string[] = [];
    const calls: [ShownTag, boolean][] = [];
    const call = async (tag: ShownTag, confirmed: boolean) => { calls.push([tag, confirmed]); };
    const no = await deleteTag(v1, async (lost) => { asked.push(lost); return false; }, call);
    expect(no).toBe('kept');
    expect(calls).toEqual([]);
    const yes = await deleteTag(v1, async () => true, call);
    expect(yes).toBe('deleted');
    expect(calls).toEqual([[v1, true]]);
    expect(asked).toEqual(['the tag v1 and its message']);
  });
});
