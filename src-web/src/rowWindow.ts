// Which rows the graph asks the host for (ADR-0031).
//
// The host keeps the history; the web interface holds only the window of rows it draws. A window
// reaches one screen beyond the view on either side, so scrolling within it needs no round trip,
// and a new one is asked for once the view leaves the rows asked for. Deciding that is plain
// arithmetic over row numbers, so it lives here and is tested directly.

/** A run of rows, `rows` long from `top`. */
export interface RowRange {
  top: number;
  rows: number;
}

/**
 * The window to ask for so the rows `first..=last` of a history of `rowCount` rows are covered,
 * or null when `asked` covers them already. The window reaches `margin` rows beyond the view on
 * either side; the end of the history needs no covering beyond itself.
 */
export function windowToAsk(
  asked: RowRange | null,
  first: number,
  last: number,
  rowCount: number,
  margin: number,
): RowRange | null {
  const end = Math.min(last, rowCount - 1);
  if (end < first) return null;
  const covered =
    asked !== null &&
    asked.top <= first &&
    Math.min(asked.top + asked.rows, rowCount) > end;
  if (covered) return null;
  const top = Math.max(0, first - margin);
  return { top, rows: end + margin + 1 - top };
}
