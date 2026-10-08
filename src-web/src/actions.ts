// The operations of G-5 a commit offers on the checked-out branch, as the core decides them
// (ADR-0033), for the commit detail to draw. `actions.test.ts` holds these to the core's answers.
import type { RepoRefs } from './types.ts';
import type { Write } from './writes.ts';

/** The operations `sha` offers, each with its write, or why there are none. */
export type Actions = { actions: Write[] } | { reason: string };

/** The operations the commit `sha` offers, in the order the detail lists them. The checked-out
    commit offers only a revert and a reset; a merge is named after a local branch at the commit
    other than the checked-out one; a reset is mixed until the developer chooses. */
export function historyActions(refs: RepoRefs, sha: string): Actions {
  const head = refs.locals.find(b => b.isHead);
  if (!head) return { reason: 'No branch is checked out' };
  const base = { branch: head.name, tip: head.sha };
  const actions: Write[] = [];
  if (head.sha !== sha) {
    const name = refs.locals.find(b => b.sha === sha && !b.isHead)?.name ?? null;
    actions.push(
      { kind: 'merge', ...base, commit: sha, name },
      { kind: 'rebase', ...base, onto: sha },
      { kind: 'cherryPick', ...base, commit: sha },
    );
  }
  actions.push(
    { kind: 'revert', ...base, commit: sha },
    { kind: 'reset', ...base, commit: sha, mode: 'mixed' },
  );
  return { actions };
}
