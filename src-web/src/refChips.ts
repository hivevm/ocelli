// What the ref chips of the selected commit offer: switching to a branch and deleting a branch or
// a tag (ADR-0031, ADR-0033). A chip carries only its text, `main`, `origin/main`, or `🏷 v1`, and
// a remote's name may hold a slash, so the text alone does not say which ref it names: remotes
// `a` and `a/b` with branches `b/c` and `c` both show `a/b/c`. Each chip is matched to a ref on
// the commit that no chip before it took, and draws what that ref offers.

import type { LocalBranch, RepoRefs, Tag } from './types.ts';
import type { Write } from './writes.ts';

/** How a chip is drawn: the graph's ref column draws the same text the same way. */
export type ChipKind = 'local' | 'remote' | 'tag';

/** One control of a chip: the label the button reads as, and the write it hands the host. */
export interface ChipOffer {
  action: 'switch' | 'delete';
  label: string;
  write: Write;
}

/** A chip of the commit, as the core's ref text names it, with what it offers. */
export interface Chip {
  ref: string;
  kind: ChipKind;
  offers: ChipOffer[];
}

/** The kind a chip's text is drawn as. */
export function chipKind(ref: string): ChipKind {
  if (ref.startsWith('🏷')) return 'tag';
  return ref.includes('/') ? 'remote' : 'local';
}

type RemoteOn = { remote: string; branch: string; tip: string };

/**
 * The chips of the commit `sha`, one for each text of `chips` in its order, each offering what
 * the ref it names offers: a tag its deletion, a local branch switching to it and its deletion
 * unless it is checked out, a remote branch switching to a local branch tracking it. A chip whose
 * ref the refs do not hold on `sha` offers nothing.
 */
export function commitChips(
  chips: string[],
  refs: Pick<RepoRefs, 'locals' | 'remotes' | 'tags'> | null,
  sha: string,
): Chip[] {
  const tags: Tag[] = (refs?.tags ?? []).filter(t => t.sha === sha);
  const locals: LocalBranch[] = (refs?.locals ?? []).filter(b => b.sha === sha);
  const remotes: RemoteOn[] = (refs?.remotes ?? []).flatMap(r => r.branches
    .filter(b => b.sha === sha)
    .map(b => ({ remote: r.name, branch: b.name, tip: b.sha })));

  /** Takes the first ref of `pool` that `matches`, so that no second chip takes it. */
  function take<T>(pool: T[], matches: (item: T) => boolean): T | undefined {
    const i = pool.findIndex(matches);
    return i < 0 ? undefined : pool.splice(i, 1)[0];
  }

  return chips.map((ref): Chip => {
    const kind = chipKind(ref);
    if (kind === 'tag') {
      const tag = take(tags, t => `🏷 ${t.name}` === ref);
      return {
        ref, kind, offers: tag ? [{
          action: 'delete',
          label: `Delete the tag ${tag.name}`,
          write: { kind: 'deleteTag', name: tag.name, commit: tag.sha, annotated: tag.annotated },
        }] : [],
      };
    }
    const local = take(locals, b => b.name === ref);
    if (local) {
      return {
        ref, kind, offers: local.isHead ? [] : [
          {
            action: 'switch',
            label: `Switch to ${local.name}`,
            write: { kind: 'switchBranch', name: local.name, tip: local.sha },
          },
          {
            action: 'delete',
            label: `Delete the branch ${local.name}`,
            write: { kind: 'deleteBranch', name: local.name, tip: local.sha, merged: local.merged },
          },
        ],
      };
    }
    const remote = take(remotes, r => `${r.remote}/${r.branch}` === ref);
    return {
      ref, kind, offers: remote ? [{
        action: 'switch',
        label: `Switch to a local branch tracking ${ref}`,
        write: { kind: 'trackRemote', ...remote },
      }] : [],
    };
  });
}
