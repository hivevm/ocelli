// Verifies: ADR-0031, G-15
import { describe, expect, test } from 'bun:test';
import { EditorState, type TransactionSpec } from '@codemirror/state';
import { history, redo, undo } from '@codemirror/commands';
import { blockLineKinds, blocksField, drawn, takeSide, toggledBase, undoableTakes } from './mergeBlocks.ts';
import type { MergeBlock } from './types.ts';

const text = 'a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> side\nm\n'
  + '<<<<<<< HEAD\no2\n||||||| base\nb2\n=======\nt2\n>>>>>>> side';
const blocks: MergeBlock[] = [
  { firstLine: 2, lastLine: 6, ours: 'ours\n', theirs: 'theirs\n', base: 'b\n', baseInFile: false },
  { firstLine: 8, lastLine: 14, ours: 'o2\n', theirs: 't2\n', base: 'b2\n', baseInFile: true },
];

function editor() {
  const field = blocksField(blocks);
  let state = EditorState.create({ doc: text, extensions: [history(), field, undoableTakes(field)] });
  const run = (command: typeof undo) => command({ state, dispatch: (tr) => { state = tr.state; } });
  return {
    field,
    get state() { return state; },
    dispatch: (spec: TransactionSpec | null) => { if (!spec) throw new Error('no block to take'); state = state.update(spec).state; },
    undo: () => run(undo),
    redo: () => run(redo),
  };
}

describe('taking a side', () => {
  test('replaces the block by ours, theirs, or both, ours first', () => {
    for (const [side, result] of [
      ['ours', 'a\nours\nm\n'], ['theirs', 'a\ntheirs\nm\n'], ['both', 'a\nours\ntheirs\nm\n'],
    ] as const) {
      const e = editor();
      e.dispatch(takeSide(e.state, e.field, 0, side));
      expect(e.state.doc.toString().startsWith(result)).toBe(true);
      expect(e.state.field(e.field)[0].taken).toBe(true);
    }
  });

  test('of the last block, at the end of a file without a final line ending, adds none', () => {
    const e = editor();
    e.dispatch(takeSide(e.state, e.field, 1, 'theirs'));
    expect(e.state.doc.toString().endsWith('m\nt2')).toBe(true);
  });

  test('undo puts the block back, and redo takes it again', () => {
    const e = editor();
    const before = blockLineKinds(e.state, e.state.field(e.field));
    e.dispatch(takeSide(e.state, e.field, 0, 'ours'));
    expect(blockLineKinds(e.state, e.state.field(e.field)).get(2)).toBeUndefined();
    e.undo();
    expect(e.state.doc.toString()).toBe(text);
    expect(e.state.field(e.field)[0].taken).toBe(false);
    expect(blockLineKinds(e.state, e.state.field(e.field))).toEqual(before);
    e.redo();
    expect(e.state.doc.toString().startsWith('a\nours\nm\n')).toBe(true);
    expect(e.state.field(e.field)[0].taken).toBe(true);
  });

  test('a block after one taken moves along and is still taken whole', () => {
    const e = editor();
    e.dispatch(takeSide(e.state, e.field, 0, 'both'));
    e.dispatch(takeSide(e.state, e.field, 1, 'ours'));
    expect(e.state.doc.toString()).toBe('a\nours\ntheirs\nm\no2');
  });
});

describe('the blocks as edited', () => {
  test('a side edited inside its block is taken as edited', () => {
    const e = editor();
    e.dispatch({ changes: { from: e.state.doc.line(3).from, to: e.state.doc.line(3).to, insert: 'fixed' } });
    e.dispatch(takeSide(e.state, e.field, 0, 'both'));
    expect(e.state.doc.toString().startsWith('a\nfixed\ntheirs\nm\n')).toBe(true);
  });

  test('a line put in before a marker line leaves the block whole', () => {
    for (const line of [2, 4, 6]) {
      const e = editor();
      e.dispatch({ changes: { from: e.state.doc.line(line).from, insert: 'x\n' } });
      const kinds = blockLineKinds(e.state, e.state.field(e.field));
      expect(kinds.get(line + 1)).toBe('marker');
    }
  });

  test('a block an edit left no block offers no side to take', () => {
    const e = editor();
    const sep = e.state.doc.line(4);
    e.dispatch({ changes: { from: sep.from, to: sep.to + 1 } });
    expect(takeSide(e.state, e.field, 0, 'ours')).toBeNull();
  });

  test('a line typed into a side is of that side', () => {
    const e = editor();
    e.dispatch({ changes: { from: e.state.doc.line(3).to, insert: '\nmore' } });
    const kinds = blockLineKinds(e.state, e.state.field(e.field));
    expect([3, 4, 5, 6, 7].map((n) => kinds.get(n))).toEqual(['ours', 'ours', 'marker', 'theirs', 'marker']);
  });

  test('a block whose marker line was deleted is no block any more', () => {
    const e = editor();
    const sep = e.state.doc.line(4);
    e.dispatch({ changes: { from: sep.from, to: sep.to + 1 } });
    expect(blockLineKinds(e.state, e.state.field(e.field)).get(2)).toBeUndefined();
  });

  test('a base opens and folds again', () => {
    const e = editor();
    e.dispatch({ effects: toggledBase.of(1) });
    expect(e.state.field(e.field)[1].baseOpen).toBe(true);
    e.dispatch({ effects: toggledBase.of(1) });
    expect(e.state.field(e.field)[1].baseOpen).toBe(false);
  });
});

describe('drawn', () => {
  test('each block has its bar, its base folded: in place where the file holds it', () => {
    const e = editor();
    const doc = e.state.doc;
    expect(drawn(e.state, e.state.field(e.field))).toEqual([
      { kind: 'bar', index: 0, at: doc.line(2).from, base: true, open: false },
      { kind: 'bar', index: 1, at: doc.line(8).from, base: true, open: false },
      { kind: 'fold', from: doc.line(10).from, to: doc.line(11).to },
    ]);
  });

  test('a base opened shows in place, or beneath its block where it came from memory', () => {
    const e = editor();
    e.dispatch({ effects: [toggledBase.of(0), toggledBase.of(1)] });
    const doc = e.state.doc;
    expect(drawn(e.state, e.state.field(e.field))).toEqual([
      { kind: 'bar', index: 0, at: doc.line(2).from, base: true, open: true },
      { kind: 'base', at: doc.line(6).to, text: 'b\n' },
      { kind: 'bar', index: 1, at: doc.line(8).from, base: true, open: true },
    ]);
  });

  test('a block without a base offers none, an empty base folds its marker line alone', () => {
    const field = blocksField([
      { firstLine: 1, lastLine: 3, ours: '', theirs: '', base: null, baseInFile: false },
      { firstLine: 4, lastLine: 7, ours: '', theirs: '', base: '', baseInFile: true },
    ]);
    const state = EditorState.create({
      doc: '<<<<<<< a\n=======\n>>>>>>> b\n<<<<<<< a\n||||||| c\n=======\n>>>>>>> b',
      extensions: [field],
    });
    expect(drawn(state, state.field(field))).toEqual([
      { kind: 'bar', index: 0, at: 0, base: false, open: false },
      { kind: 'bar', index: 1, at: state.doc.line(4).from, base: true, open: false },
      { kind: 'fold', from: state.doc.line(5).from, to: state.doc.line(5).to },
    ]);
  });
});
