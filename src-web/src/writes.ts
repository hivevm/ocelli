// The web interface's side of a write (ADR-0031): the writes it hands the host, asking before one
// that discards work in the words the core names it with (ADR-0033), and running one at a time.
//
// All of it is bookkeeping over strings and numbers, so it lives here and is tested directly.

import type { Conflict, FileChange, Operation } from './types.ts';

/** A write as the core's `Write` crosses to the host, its kind in `kind`. */
export type Write =
  | { kind: 'createTag'; name: string; commit: string; message: string | null }
  | { kind: 'deleteTag'; name: string; commit: string; annotated: boolean }
  | { kind: 'createStash'; message: string | null; untracked: boolean }
  | { kind: 'applyStash'; index: number; commit: string; drop: boolean }
  | { kind: 'dropStash'; index: number; commit: string }
  | { kind: 'stageFile'; path: string; shown: string | null }
  | { kind: 'unstageFile'; path: string; shown: string | null }
  | { kind: 'commit'; message: string; head: string | null }
  | { kind: 'createBranch'; name: string; commit: string; switch: boolean }
  | { kind: 'switchBranch'; name: string; tip: string }
  | { kind: 'detach'; commit: string }
  | { kind: 'trackRemote'; remote: string; branch: string; tip: string }
  | { kind: 'deleteBranch'; name: string; tip: string; merged: boolean }
  | { kind: 'fetch'; remote: string }
  | { kind: 'pull'; branch: string; tip: string }
  | { kind: 'push'; branch: string; tip: string }
  | { kind: 'merge'; branch: string; tip: string; commit: string; name: string | null }
  | { kind: 'rebase'; branch: string; tip: string; onto: string }
  | { kind: 'cherryPick'; branch: string; tip: string; commit: string }
  | { kind: 'revert'; branch: string; tip: string; commit: string }
  | { kind: 'reset'; branch: string; tip: string; commit: string; mode: ResetMode }
  | { kind: 'resolve'; path: string; conflict: Conflict; take: Take }
  | { kind: 'saveMerge'; path: string; conflict: Conflict; opened: string; result: string }
  | { kind: 'continue'; operation: Operation; head: string }
  | { kind: 'abort'; operation: Operation; head: string };

/** How a reset moves the checked-out branch, as git reset names it. */
export type ResetMode = 'soft' | 'mixed' | 'hard';

/** Which side of a conflicted file a resolution takes. */
export type Take = 'ours' | 'theirs' | 'asEdited';

/** The name of an operation as git writes it. */
export function operationName(operation: Operation): string {
  return operation === 'cherryPick' ? 'cherry-pick' : operation;
}

/** A write of one kind. */
export type WriteOf<K extends Write['kind']> = Extract<Write, { kind: K }>;

/** The tag the form describes, or null while it has no name. A blank message makes a
    lightweight tag. */
export function tagRequest(sha: string, name: string, message: string): WriteOf<'createTag'> | null {
  const trimmed = name.trim();
  if (trimmed === '') return null;
  return { kind: 'createTag', name: trimmed, commit: sha, message: blankIsNone(message) };
}

/** The stash the dialog describes. */
export function stashRequest(message: string, untracked: boolean): WriteOf<'createStash'> {
  return { kind: 'createStash', message: blankIsNone(message), untracked };
}

/** What the stage toggle of a working-directory file asks for: a staged file is unstaged, one
    staged in part or not at all is staged whole, each expecting the content shown; nothing for a
    file that does not stage, a commit's. */
export function stageToggle(file: FileChange): Write | null {
  const { path, staging } = file;
  if (!staging) return null;
  return staging.staged === 'yes'
    ? { kind: 'unstageFile', path, shown: staging.shown.index }
    : { kind: 'stageFile', path, shown: staging.shown.workdir };
}

/** The branch the form describes at `commit`, or null while it has no name. */
export function branchRequest(
  commit: string, name: string, switchTo: boolean,
): WriteOf<'createBranch'> | null {
  const trimmed = name.trim();
  return trimmed === '' ? null : { kind: 'createBranch', name: trimmed, commit, switch: switchTo };
}

/** The commit the form describes on `head`, or null while it has no message. */
export function commitRequest(head: string | null, message: string): WriteOf<'commit'> | null {
  return message.trim() === '' ? null : { kind: 'commit', message, head };
}

function blankIsNone(text: string): string | null {
  return text.trim() === '' ? null : text;
}

/** The label of the button that goes ahead with a write that discards work. */
export function actionLabel(write: Write): string {
  switch (write.kind) {
    case 'deleteTag': return 'Delete tag';
    case 'applyStash': return write.drop ? 'Pop stash' : 'Apply stash';
    case 'dropStash': return 'Drop stash';
    case 'deleteBranch': return 'Delete branch';
    case 'reset': return 'Reset hard';
    case 'abort': return `Abort the ${operationName(write.operation)}`;
    case 'resolve': return write.take === 'ours' ? 'Take ours' : 'Take theirs';
    default: return 'Go ahead';
  }
}

/** How a write ended: kept on a decline, or written, with the commit it made where it made one. */
export type Outcome = { kind: 'kept' } | { kind: 'written'; made: string | null };

/**
 * Performs `write`: `discards` asks the core what it discards, and where it discards anything,
 * `ask` puts the question naming it and resolves with the answer; `call` asks the host to write,
 * confirmed where the developer said yes, and resolves with the commit the write made. On a
 * decline nothing is written.
 */
export async function confirmedWrite(
  write: Write,
  discards: (write: Write) => Promise<string | null>,
  ask: (lost: string, write: Write) => Promise<boolean>,
  call: (write: Write, confirmed: boolean) => Promise<string | null>,
): Promise<Outcome> {
  const lost = await discards(write);
  if (lost !== null && !(await ask(lost, write))) return { kind: 'kept' };
  const made = await call(write, lost !== null);
  return { kind: 'written', made };
}

/** A write's refusal, or null for a write done or kept. */
export interface Answer {
  error: string | null;
}

/**
 * The one write the commit detail runs at a time, for the commit it was asked for: a second is
 * not started while one runs, from whatever control, and the answer is dropped when another
 * commit is selected by the time it comes.
 */
export class OneWrite {
  #running = false;

  /** Whether a write runs. */
  get running(): boolean {
    return this.#running;
  }

  /**
   * Runs `write` for the commit `sha` and resolves with its answer, or with null when it was not
   * started or `selected` names another commit once it answers.
   */
  async run(
    sha: string,
    write: () => Promise<string | null>,
    selected: () => string | undefined,
  ): Promise<Answer | null> {
    if (this.#running) return null;
    this.#running = true;
    try {
      const error = await write();
      return selected() === sha ? { error } : null;
    } finally {
      this.#running = false;
    }
  }
}
