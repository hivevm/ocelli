// The merge editor's decisions that need no editor (ADR-0031, G-15): which lines of the file
// the blocks the core found mark, whether the result is still unsaved, and the write that saves it.
import type { Conflict, MergeBlock, MergeFile } from './types.ts';
import type { WriteOf } from './writes.ts';

/** What a line of a block is: one of its markers, or a line of ours, the base, or theirs. */
export type BlockLine = 'marker' | 'ours' | 'base' | 'theirs';

/** The number of lines in a side's text, each ending in a line ending. */
function lineCount(text: string): number {
  return text === '' ? 0 : text.split('\n').length - (text.endsWith('\n') ? 1 : 0);
}

/** What each line of the blocks is, by line number counted from 1: the markers `<<<<<<<`,
    `|||||||`, `=======`, and `>>>>>>>`, and the lines of ours, the base, and theirs between. */
export function blockLines(blocks: MergeBlock[]): Map<number, BlockLine> {
  const lines = new Map<number, BlockLine>();
  for (const block of blocks) {
    let line = block.firstLine;
    const run = (kind: BlockLine, count: number) => {
      for (let i = 0; i < count; i++) lines.set(line++, kind);
    };
    run('marker', 1);
    run('ours', lineCount(block.ours));
    if (block.base !== null) {
      run('marker', 1);
      run('base', lineCount(block.base));
    }
    run('marker', 1);
    run('theirs', lineCount(block.theirs));
    lines.set(block.lastLine, 'marker');
  }
  return lines;
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
