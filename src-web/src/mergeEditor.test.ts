// Verifies: ADR-0031, ADR-0033, G-15, Q-7
import { describe, expect, test } from 'bun:test';
import { blockLines, saveRequest, unsaved, unsavedLoss } from './mergeEditor.ts';
import type { Conflict, MergeFile } from './types.ts';

const file: MergeFile = {
  text: 'a\r\n<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> side\r\nz\r\n',
  blob: 'b10b',
  blocks: [{ firstLine: 2, lastLine: 6, ours: 'ours\r\n', theirs: 'theirs\r\n', base: null }],
};

describe('blockLines', () => {
  test('each block marks its markers and the lines of each side', () => {
    expect([...blockLines(file.blocks)]).toEqual([
      [2, 'marker'], [3, 'ours'], [4, 'marker'], [5, 'theirs'], [6, 'marker'],
    ]);
  });

  test('a block with a base marks the base between ours and theirs, an empty base none', () => {
    const lines = blockLines([
      { firstLine: 1, lastLine: 8, ours: 'o\n', theirs: 't\n', base: 'b1\nb2\n' },
      { firstLine: 9, lastLine: 14, ours: 'o\n', theirs: 't\n', base: '' },
    ]);
    expect([...lines]).toEqual([
      [1, 'marker'], [2, 'ours'], [3, 'marker'], [4, 'base'], [5, 'base'], [6, 'marker'],
      [7, 'theirs'], [8, 'marker'],
      [9, 'marker'], [10, 'ours'], [11, 'marker'], [12, 'marker'], [13, 'theirs'], [14, 'marker'],
    ]);
  });
});

describe('leaving the editor', () => {
  test('the text as opened, its line endings the editor\'s, is saved; any edit is not', () => {
    expect(unsaved(file, file.text.replace(/\r\n/g, '\n'))).toBe(false);
    expect(unsaved(file, 'a\nboth\nz\n')).toBe(true);
  });

  test('the question names the result that leaving discards', () => {
    expect(unsavedLoss('src/f.rs')).toBe('the result not saved for src/f.rs');
  });
});

describe('saveRequest', () => {
  test('the result is saved expecting the conflict and the file it was opened on', () => {
    const conflict: Conflict = { path: 'f', ancestor: 'a', ours: 'o', theirs: 't' };
    expect(saveRequest(conflict, file, 'a\nboth\nz\n')).toEqual({
      kind: 'saveMerge', path: 'f', conflict, opened: 'b10b', result: 'a\nboth\nz\n',
    });
  });
});
