// The transfers of the checked-out branch as the core decides them (ADR-0035), for the toolbar to
// draw: which fetch, pull, and push the refs make possible, or why there is none, and whether a
// write can be stopped while it runs. `transfers.test.ts` holds these to the core's answers.
import type { RepoRefs } from './types.ts';
import type { Write, WriteOf } from './writes.ts';

/** A transfer the refs make possible, or why there is none. */
export type Answer<K extends Write['kind']> = { write: WriteOf<K> } | { reason: string };

/** The fetch of the default remote. */
export function fetchOf(refs: RepoRefs): Answer<'fetch'> {
  const remote = refs.defaultRemote;
  return remote ? { write: { kind: 'fetch', remote } } : { reason: 'No remote to fetch' };
}

/** The pull of the checked-out branch from its upstream. */
export function pullOf(refs: RepoRefs): Answer<'pull'> {
  const head = refs.locals.find(b => b.isHead);
  if (!head) return { reason: 'No branch is checked out to pull' };
  if (head.upstream === null) return { reason: `${head.name} has no upstream to pull from` };
  return { write: { kind: 'pull', branch: head.name, tip: head.sha } };
}

/** The push of the checked-out branch to its upstream, or to the default remote. */
export function pushOf(refs: RepoRefs): Answer<'push'> {
  const head = refs.locals.find(b => b.isHead);
  if (!head) return { reason: 'No branch is checked out to push' };
  if (head.upstream === null && !refs.defaultRemote) {
    return { reason: `There is no remote to push ${head.name} to` };
  }
  return { write: { kind: 'push', branch: head.name, tip: head.sha } };
}

/** Whether `write` can be stopped while it runs: a fetch, and a pull while it fetches. libgit2
    cannot stop a push once its objects are sent. */
export function canStop(write: Write): boolean {
  return write.kind === 'fetch' || write.kind === 'pull';
}
