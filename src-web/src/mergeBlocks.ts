// The merge editor's blocks in CodeMirror's state (ADR-0031, G-15): where each block's markers
// are now, moved along with the edits around them, whether a side was taken for it, and whether
// its base is open. Taking a side is one change with an effect undo inverts, so undo puts the
// block back as it was.
import { StateEffect, StateField, type EditorState, type TransactionSpec } from '@codemirror/state';
import { invertedEffects } from '@codemirror/commands';
import { lineKinds, markerLines, sideText, type BlockLine, type Side } from './mergeEditor.ts';
import type { MergeBlock } from './types.ts';

/** A block as the editor holds it: its markers' line starts in the text as it is now, each kept
    with its own line where a line is put in before it. */
export interface BlockState {
  block: MergeBlock;
  markers: number[];
  taken: boolean;
  baseOpen: boolean;
}

/** A side was taken for the block `index`. */
const taken = StateEffect.define<number>();

/** The block `index` is back with its markers where they were before a side was taken. */
const restored = StateEffect.define<{ index: number; markers: number[] }>({
  map: (value, changes) => ({ ...value, markers: value.markers.map((p) => changes.mapPos(p, 1)) }),
});

/** The base of the block `index` opens, or folds again. */
export const toggledBase = StateEffect.define<number>();

/** The blocks of the file the editor opened, as the field `blocks` holds them. */
export function blocksField(opened: MergeBlock[]) {
  return StateField.define<BlockState[]>({
    create: (state) => opened.map((block) => ({
      block,
      markers: markerLines(block).map((line) => state.doc.line(line).from),
      taken: false,
      baseOpen: false,
    })),
    update(blocks, tr) {
      let next = tr.docChanged
        ? blocks.map((b) => ({ ...b, markers: b.markers.map((p) => tr.changes.mapPos(p, 1)) }))
        : blocks;
      for (const effect of tr.effects) {
        const set = (index: number, change: Partial<BlockState>) => {
          next = next.map((b, i) => (i === index ? { ...b, ...change } : b));
        };
        if (effect.is(taken)) set(effect.value, { taken: true });
        if (effect.is(restored)) set(effect.value.index, { taken: false, markers: effect.value.markers });
        if (effect.is(toggledBase)) set(effect.value, { baseOpen: !next[effect.value].baseOpen });
      }
      return next;
    },
  });
}

/** The effects undoing a transaction applies to the blocks: a block taken comes back with the
    markers it had, and redoing takes it again. */
export function undoableTakes(field: StateField<BlockState[]>) {
  return invertedEffects.of((tr) => tr.effects.flatMap((effect): StateEffect<unknown>[] => {
    if (effect.is(taken)) {
      return [restored.of({ index: effect.value, markers: tr.startState.field(field)[effect.value].markers })];
    }
    return effect.is(restored) ? [taken.of(effect.value.index)] : [];
  }));
}

/** The lines a block's markers are on now, or `null` where an edit left it no block: a marker
    line gone or moved out of order. */
export function intactMarkers(state: EditorState, block: BlockState): number[] | null {
  if (block.taken) return null;
  const prefixes = block.markers.length === 4
    ? ['<<<<<<<', '|||||||', '=======', '>>>>>>>']
    : ['<<<<<<<', '=======', '>>>>>>>'];
  const lines = block.markers.map((p) => state.doc.lineAt(p));
  const intact = lines.every((line, i) => line.from === block.markers[i]
    && line.text.startsWith(prefixes[i]) && (i === 0 || line.number > lines[i - 1].number));
  return intact ? lines.map((line) => line.number) : null;
}

/** What each line of the intact blocks is now, by line number. */
export function blockLineKinds(state: EditorState, blocks: BlockState[]): Map<number, BlockLine> {
  const kinds = new Map<number, BlockLine>();
  for (const block of blocks) {
    const markers = intactMarkers(state, block);
    if (markers) for (const [line, kind] of lineKinds(markers)) kinds.set(line, kind);
  }
  return kinds;
}

/** What a block's bar, its fold, and its base beneath draw, where the decorations go. */
export type Drawn =
  | { kind: 'bar'; index: number; at: number; base: boolean; open: boolean }
  | { kind: 'fold'; from: number; to: number }
  | { kind: 'base'; at: number; text: string };

/** What the intact blocks draw: the bar above each, and its base folded in place where the file
    holds it, or opened beneath the block where it came from memory. */
export function drawn(state: EditorState, blocks: BlockState[]): Drawn[] {
  const found: Drawn[] = [];
  blocks.forEach((block, index) => {
    const markers = intactMarkers(state, block);
    if (!markers) return;
    const base = block.block.base;
    const at = state.doc.line(markers[0]).from;
    found.push({ kind: 'bar', index, at, base: base !== null, open: block.baseOpen });
    if (base === null) return;
    if (markers.length === 4 && !block.baseOpen) {
      found.push({ kind: 'fold', from: state.doc.line(markers[1]).from, to: state.doc.line(markers[2] - 1).to });
    } else if (markers.length === 3 && block.baseOpen) {
      found.push({ kind: 'base', at: state.doc.line(markers[2]).to, text: base });
    }
  });
  return found;
}

/** The text of the lines after `first` and before `next`, each with its line ending. */
function between(state: EditorState, first: number, next: number): string {
  return state.doc.sliceString(state.doc.line(first).to + 1, state.doc.line(next).from);
}

/** The transaction that replaces the block `index`, its markers included, by `side` as the
    block holds it now; none where an edit left it no block. */
export function takeSide(
  state: EditorState,
  field: StateField<BlockState[]>,
  index: number,
  side: Side,
): TransactionSpec | null {
  const block = state.field(field)[index];
  const markers = intactMarkers(state, block);
  if (!markers) return null;
  const sides = {
    ours: between(state, markers[0], markers[1]),
    theirs: between(state, markers[markers.length - 2], markers[markers.length - 1]),
  };
  const from = state.doc.line(markers[0]).from;
  const last = state.doc.line(markers[markers.length - 1]);
  const ended = last.to < state.doc.length;
  let insert = sideText(sides, side);
  if (!ended && insert.endsWith('\n')) insert = insert.slice(0, -1);
  return {
    changes: { from, to: ended ? last.to + 1 : last.to, insert },
    effects: taken.of(index),
    userEvent: 'input.merge',
  };
}
