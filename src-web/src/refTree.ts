// Grouping and filtering for the repository sidebar (ADR-0026).
//
// Branch names are paths: `feature/sidebar` and `feature/graph` belong together
// under `feature`. Turning a flat list into that tree is plain data work with no
// canvas and no repository behind it, so it lives here and is tested directly.

/** Anything the sidebar lists by a slash-separated name. */
export interface Named {
  name: string;
}

/** One folder of the name tree; the root folder carries an empty name and path. */
export interface RefFolder<T extends Named> {
  /** The last path segment — `feature` for the folder holding `feature/*`. */
  name: string;
  /** The full path down to this folder, unique among siblings and usable as a key. */
  path: string;
  folders: RefFolder<T>[];
  leaves: RefLeaf<T>[];
}

/** An entry at its position in the tree, labelled by its last path segment. */
export interface RefLeaf<T extends Named> {
  /** Last segment of the entry's name — `sidebar` for `feature/sidebar`. */
  label: string;
  item: T;
}

function emptyFolder<T extends Named>(name: string, path: string): RefFolder<T> {
  return { name, path, folders: [], leaves: [] };
}

/**
 * Groups entries into a tree by the slashes in their names. Entries without a
 * slash stay directly in the root, so a repository of flat branch names produces
 * a flat list rather than a tree of one-child folders.
 */
export function groupByFolder<T extends Named>(items: T[]): RefFolder<T> {
  const root = emptyFolder<T>('', '');

  for (const item of items) {
    // A leading, trailing or doubled slash would otherwise create nameless
    // folders; dropping empty segments keeps the tree navigable.
    const segments = item.name.split('/').filter(s => s.length > 0);
    if (segments.length === 0) continue;

    const label = segments[segments.length - 1];
    let folder = root;
    for (let i = 0; i < segments.length - 1; i++) {
      const path = folder.path === '' ? segments[i] : `${folder.path}/${segments[i]}`;
      let next = folder.folders.find(f => f.name === segments[i]);
      if (!next) {
        next = emptyFolder<T>(segments[i], path);
        folder.folders.push(next);
      }
      folder = next;
    }
    folder.leaves.push({ label, item });
  }

  return root;
}

/**
 * Case-insensitive substring match over the full name, so a query keeps entries
 * findable by their folder as well as by their last segment. An empty or
 * whitespace-only query matches everything.
 */
export function filterByName<T extends Named>(items: T[], query: string): T[] {
  const needle = query.trim().toLowerCase();
  if (needle === '') return items;
  return items.filter(item => item.name.toLowerCase().includes(needle));
}

/** Total number of entries in a folder and everything below it. */
export function countLeaves<T extends Named>(folder: RefFolder<T>): number {
  return folder.leaves.length + folder.folders.reduce((sum, f) => sum + countLeaves(f), 0);
}

/**
 * Whether any entry in a folder or below it satisfies `pred`.
 *
 * The checked-out branch is what this exists for: `feat/sidebar` sits inside a
 * folder, and a collapsed folder would otherwise hide the one row the user most
 * needs to find. Marking the folders on the way down keeps it visible.
 */
export function anyLeaf<T extends Named>(
  folder: RefFolder<T>,
  pred: (item: T) => boolean,
): boolean {
  return (
    folder.leaves.some(leaf => pred(leaf.item)) || folder.folders.some(f => anyLeaf(f, pred))
  );
}
