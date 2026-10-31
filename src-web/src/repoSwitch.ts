// Why a sidebar row that switches to another repository cannot (ADR-0028, ADR-0037): the working
// tree on screen, a submodule never checked out, or a repository outside the server root, whose
// path the remote server sends empty so that the row shows without its switch.

import type { ParentRepo, Submodule, Worktree } from './types.ts';

/** The reason a row leading outside the remote server's root shows in place of its switch. */
export const OUTSIDE_ROOT = 'outside the folder ocelli-server shows — it opens no repository there';

export function worktreeBlocked(w: Pick<Worktree, 'path' | 'isCurrent'>): string | undefined {
  if (w.isCurrent) return 'the working tree you are viewing';
  if (w.path === '') return OUTSIDE_ROOT;
  return undefined;
}

export function submoduleBlocked(sub: Pick<Submodule, 'workdir' | 'state'>): string | undefined {
  if (sub.state === 'uninitialized') return 'not initialized — no working copy to open';
  if (sub.workdir === '') return OUTSIDE_ROOT;
  return undefined;
}

export function parentBlocked(parent: Pick<ParentRepo, 'path'>): string | undefined {
  return parent.path === '' ? OUTSIDE_ROOT : undefined;
}
