// Verifies: ADR-0031, ADR-0033, G-15, Q-7
import { describe, expect, test } from 'bun:test';
import { lineKinds, markerLines, saveRequest, sideText, unsaved, unsavedLoss } from './mergeEditor.ts';
import type { Conflict, MergeFile } from './types.ts';

const file: MergeFile = {
  text: 'a\r\n<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> side\r\nz\r\n',
  blob: 'b10b',
  blocks: [{ firstLine: 2, lastLine: 6, ours: 'ours\r\n', theirs: 'theirs\r\n', base: null, baseInFile: false }],
};

describe('markerLines and lineKinds', () => {
  test('each block marks its markers and the lines of each side', () => {
    expect(markerLines(file.blocks[0])).toEqual([2, 4, 6]);
    expect([...lineKinds([2, 4, 6])]).toEqual([
      [2, 'marker'], [3, 'ours'], [4, 'marker'], [5, 'theirs'], [6, 'marker'],
    ]);
  });

  test('a base in the file is marked between ours and theirs, an empty base and one from memory not', () => {
    const inFile = { firstLine: 1, lastLine: 8, ours: 'o\n', theirs: 't\n', base: 'b1\nb2\n', baseInFile: true };
    const empty = { firstLine: 9, lastLine: 14, ours: 'o\n', theirs: 't\n', base: '', baseInFile: true };
    const merged = { firstLine: 1, lastLine: 5, ours: 'o\n', theirs: 't\n', base: 'b\n', baseInFile: false };
    expect(markerLines(inFile)).toEqual([1, 3, 6, 8]);
    expect(markerLines(empty)).toEqual([9, 11, 12, 14]);
    expect(markerLines(merged)).toEqual([1, 3, 5]);
    expect([...lineKinds([1, 3, 6, 8])]).toEqual([
      [1, 'marker'], [2, 'ours'], [3, 'marker'], [4, 'base'], [5, 'base'], [6, 'marker'],
      [7, 'theirs'], [8, 'marker'],
    ]);
  });

  test('lines added inside a side are of that side', () => {
    expect([...lineKinds([1, 4, 6])]).toEqual([
      [1, 'marker'], [2, 'ours'], [3, 'ours'], [4, 'marker'], [5, 'theirs'], [6, 'marker'],
    ]);
  });
});

describe('sideText', () => {
  test('ours, theirs, or both, ours first, in the editor\'s line endings', () => {
    const block = file.blocks[0];
    expect(sideText(block, 'ours')).toBe('ours\n');
    expect(sideText(block, 'theirs')).toBe('theirs\n');
    expect(sideText(block, 'both')).toBe('ours\ntheirs\n');
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
