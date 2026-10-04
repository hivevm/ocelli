// Verifies: ADR-0020
import { describe, expect, test } from 'bun:test';
import { toSplitRows } from './diffSplit.ts';
import type { DiffLine, DiffLineKind } from './types.ts';

function line(kind: DiffLineKind, content: string, oldNo: number | null, newNo: number | null): DiffLine {
  return { kind, content, oldLineno: oldNo, newLineno: newNo };
}

describe('toSplitRows', () => {
  test('context lines appear on both sides with their own line numbers', () => {
    const rows = toSplitRows([line('context', 'same', 3, 5)]);

    expect(rows).toEqual([
      {
        kind: 'line',
        left: { kind: 'context', content: 'same', lineno: 3 },
        right: { kind: 'context', content: 'same', lineno: 5 },
      },
    ]);
  });

  test('a balanced change block pairs the i-th delete with the i-th add', () => {
    const rows = toSplitRows([
      line('delete', 'old 1', 10, null),
      line('delete', 'old 2', 11, null),
      line('add', 'new 1', null, 10),
      line('add', 'new 2', null, 11),
    ]);

    expect(rows.length).toBe(2);
    expect(rows[0]).toEqual({
      kind: 'line',
      left: { kind: 'delete', content: 'old 1', lineno: 10 },
      right: { kind: 'add', content: 'new 1', lineno: 10 },
    });
    expect(rows[1]).toEqual({
      kind: 'line',
      left: { kind: 'delete', content: 'old 2', lineno: 11 },
      right: { kind: 'add', content: 'new 2', lineno: 11 },
    });
  });

  test('the longer side of an unbalanced block runs against empty cells', () => {
    const rows = toSplitRows([
      line('delete', 'gone', 7, null),
      line('add', 'one', null, 7),
      line('add', 'two', null, 8),
    ]);

    expect(rows.length).toBe(2);
    expect(rows[0].kind === 'line' && rows[0].left.kind).toBe('delete');
    expect(rows[1]).toEqual({
      kind: 'line',
      left: { kind: 'empty', content: '', lineno: null },
      right: { kind: 'add', content: 'two', lineno: 8 },
    });
  });

  test('pure additions and pure deletions keep the other side empty', () => {
    const rows = toSplitRows([line('add', 'new file line', null, 1)]);

    expect(rows).toEqual([
      {
        kind: 'line',
        left: { kind: 'empty', content: '', lineno: null },
        right: { kind: 'add', content: 'new file line', lineno: 1 },
      },
    ]);
  });

  test('a hunk header becomes a banner and closes the open change block', () => {
    const rows = toSplitRows([
      line('delete', 'old', 1, null),
      line('hunk', '@@ -10,2 +10,2 @@', null, null),
      line('add', 'new', null, 10),
    ]);

    expect(rows.length).toBe(3);
    expect(rows[0].kind === 'line' && rows[0].right.kind).toBe('empty');
    expect(rows[1]).toEqual({ kind: 'banner', content: '@@ -10,2 +10,2 @@' });
    expect(rows[2].kind === 'line' && rows[2].left.kind).toBe('empty');
  });
});
