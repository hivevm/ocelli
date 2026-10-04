import type { NodeJson } from './types.ts';

/**
 * Rows of `start` and every loaded ancestor, following parent SHAs.
 *
 * Parents outside the loaded set (beyond the commit limit) have no node and
 * are skipped. Iterative so deep histories cannot exhaust the call stack,
 * mirroring the layout traversal on the Rust side.
 */
export function ancestryRows(start: NodeJson, nodeBySha: Map<string, NodeJson>): Set<number> {
  return walk(start, nodeBySha, (n) => n.parents);
}

/** Rows of `start` and every descendant, following child SHAs (always loaded). */
export function descendantRows(start: NodeJson, nodeBySha: Map<string, NodeJson>): Set<number> {
  return walk(start, nodeBySha, (n) => n.children);
}

/** Rows of the commit's full lineage: itself, all ancestors, all descendants. */
export function lineageRows(start: NodeJson, nodeBySha: Map<string, NodeJson>): Set<number> {
  const rows = ancestryRows(start, nodeBySha);
  for (const row of descendantRows(start, nodeBySha)) rows.add(row);
  return rows;
}

function walk(
  start: NodeJson,
  nodeBySha: Map<string, NodeJson>,
  next: (n: NodeJson) => string[],
): Set<number> {
  const rows = new Set<number>();
  const seen = new Set<string>();
  const stack: NodeJson[] = [start];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (seen.has(node.sha)) continue;
    seen.add(node.sha);
    rows.add(node.row);
    for (const sha of next(node)) {
      const neighbour = nodeBySha.get(sha);
      if (neighbour) stack.push(neighbour);
    }
  }
  return rows;
}
