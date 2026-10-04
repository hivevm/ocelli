import type { NodeJson } from './types.ts';

/**
 * Nodes matching a search query, ordered by row (newest first).
 *
 * Matching is a case-insensitive substring test over what a user sees in a
 * row: message, author, committer, SHA, and ref names. A blank query matches
 * nothing — an empty result means "no search active" to the caller.
 */
export function searchNodes(nodes: NodeJson[], query: string): NodeJson[] {
  const q = query.trim().toLowerCase();
  if (q === '') return [];
  return nodes
    .filter(
      (n) =>
        n.message.toLowerCase().includes(q) ||
        n.author.toLowerCase().includes(q) ||
        n.committer.toLowerCase().includes(q) ||
        n.sha.toLowerCase().includes(q) ||
        n.refs.some((ref) => ref.toLowerCase().includes(q)),
    )
    .sort((a, b) => a.row - b.row);
}
