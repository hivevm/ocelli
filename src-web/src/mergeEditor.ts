// The merge editor's decisions that need no editor (ADR-0031, G-15): which lines of the file
// the blocks the core found mark, what taking a side puts in a block's place, whether the result
// is still unsaved, and the write that saves it.
import type { Conflict, MergeBlock, MergeFile } from './types.ts';
import type { WriteOf } from './writes.ts';

/** What a line of a block is: one of its markers, or a line of ours, the base, or theirs. */
export type BlockLine = 'marker' | 'ours' | 'base' | 'theirs';

/** What taking a side replaces a block by: ours, theirs, or both, ours first. */
export type Side = 'ours' | 'theirs' | 'both';

/** The number of lines in a side's text, each ending in a line ending. */
function lineCount(text: string): number {
  return text === '' ? 0 : text.split('\n').length - (text.endsWith('\n') ? 1 : 0);
}

/** The lines of a block's markers as the file was opened, counted from 1: `<<<<<<<`, `|||||||`
    where the base is in the file, `=======`, and `>>>>>>>`. */
export function markerLines(block: MergeBlock): number[] {
  const opened = block.firstLine + 1 + lineCount(block.ours);
  const base = block.baseInFile && block.base !== null ? [opened] : [];
  const between = block.baseInFile && block.base !== null ? opened + 1 + lineCount(block.base) : opened;
  return [block.firstLine, ...base, between, block.lastLine];
}

/** What each line of a block is, by line number, from the lines its markers are on now: ours
    after the first, the base after `|||||||` where there are four, theirs after `=======`. */
export function lineKinds(markers: number[]): Map<number, BlockLine> {
  const sides: BlockLine[] = markers.length === 4 ? ['ours', 'base', 'theirs'] : ['ours', 'theirs'];
  const lines = new Map<number, BlockLine>();
  markers.forEach((line, i) => {
    lines.set(line, 'marker');
    for (let between = line + 1; i < sides.length && between < markers[i + 1]; between++) {
      lines.set(between, sides[i]);
    }
  });
  return lines;
}

/** The text that taking `side` puts in a block's place, from its sides as they are now, in the
    editor's `\n` line endings. */
export function sideText(sides: { ours: string; theirs: string }, side: Side): string {
  const text = side === 'ours' ? sides.ours : side === 'theirs' ? sides.theirs : sides.ours + sides.theirs;
  return text.replace(/\r\n/g, '\n');
}

/** Whether `result`, as the editor holds it with `\n` line endings, differs from the file it
    was opened on: leaving it then discards work, which the developer is asked about first. */
export function unsaved(file: MergeFile, result: string): boolean {
  return result !== file.text.replace(/\r\n/g, '\n');
}

/** What leaving the editor with a result not saved discards, in the words the question puts. */
export function unsavedLoss(path: string): string {
  return `the result not saved for ${path}`;
}

/** The write that saves `result` as the resolution of `conflict`, expecting the file it was
    opened on. */
export function saveRequest(conflict: Conflict, file: MergeFile, result: string): WriteOf<'saveMerge'> {
  return { kind: 'saveMerge', path: conflict.path, conflict, opened: file.blob, result };
}
