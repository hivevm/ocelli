import type { DiffLine } from './types.ts';

export type SplitCellKind = 'add' | 'delete' | 'context' | 'empty';

export interface SplitCell {
  kind: SplitCellKind;
  content: string;
  lineno: number | null;
}

/** One row of the side-by-side view: either a full-width banner (hunk header
    or meta line) or a line with an old-file cell left and a new-file cell right. */
export type SplitRow =
  | { kind: 'banner'; content: string }
  | { kind: 'line'; left: SplitCell; right: SplitCell };

const EMPTY: SplitCell = { kind: 'empty', content: '', lineno: null };

/**
 * Folds a unified diff into side-by-side rows.
 *
 * Within a change block (consecutive deletes followed by consecutive adds,
 * as git emits them), the i-th deleted line is paired with the i-th added
 * line; the longer side runs against empty cells. Context lines appear on
 * both sides, hunk headers and meta lines become full-width banners.
 */
export function toSplitRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let dels: DiffLine[] = [];
  let adds: DiffLine[] = [];

  function flush(): void {
    const n = Math.max(dels.length, adds.length);
    for (let i = 0; i < n; i++) {
      rows.push({
        kind: 'line',
        left: dels[i]
          ? { kind: 'delete', content: dels[i].content, lineno: dels[i].oldLineno }
          : EMPTY,
        right: adds[i]
          ? { kind: 'add', content: adds[i].content, lineno: adds[i].newLineno }
          : EMPTY,
      });
    }
    dels = [];
    adds = [];
  }

  for (const line of lines) {
    if (line.kind === 'delete') {
      dels.push(line);
    } else if (line.kind === 'add') {
      adds.push(line);
    } else {
      flush();
      if (line.kind === 'context') {
        rows.push({
          kind: 'line',
          left: { kind: 'context', content: line.content, lineno: line.oldLineno },
          right: { kind: 'context', content: line.content, lineno: line.newLineno },
        });
      } else {
        rows.push({ kind: 'banner', content: line.content });
      }
    }
  }
  flush();
  return rows;
}
