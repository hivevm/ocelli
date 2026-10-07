// The web interface's side of a write (ADR-0033): the writes it hands the host, asking before one
// that discards work in the words the core names it with (ADR-0034), and running one at a time.
//
// All of it is bookkeeping over strings and numbers, so it lives here and is tested directly.

import type { FileChange } from './types.ts';

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
  | { kind: 'deleteBranch'; name: string; tip: string; merged: boolean };

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
    staged in part or not at all is staged whole, each expecting the content shown. */
export function stageToggle(file: FileChange): Write {
  const { path, shown } = file;
  return file.staged === 'yes'
    ? { kind: 'unstageFile', path, shown: shown?.index ?? null }
    : { kind: 'stageFile', path, shown: shown?.workdir ?? null };
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
