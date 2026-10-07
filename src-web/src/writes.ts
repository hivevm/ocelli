// The web interface's side of a write (ADR-0033): the writes it hands the host, asking before one
// that discards work in the words the core names it with (ADR-0034), and running one at a time.
//
// All of it is bookkeeping over strings and numbers, so it lives here and is tested directly.

/** A write as the core's `Write` crosses to the host, its kind in `kind`. */
export type Write =
  | { kind: 'createTag'; name: string; commit: string; message: string | null }
  | { kind: 'deleteTag'; name: string; commit: string; annotated: boolean }
  | { kind: 'createStash'; message: string | null; untracked: boolean }
  | { kind: 'applyStash'; index: number; commit: string; drop: boolean }
  | { kind: 'dropStash'; index: number; commit: string };

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

function blankIsNone(text: string): string | null {
  return text.trim() === '' ? null : text;
}

/** The label of the button that goes ahead with a write that discards work. */
export function actionLabel(write: Write): string {
  switch (write.kind) {
    case 'deleteTag': return 'Delete tag';
    case 'applyStash': return write.drop ? 'Pop stash' : 'Apply stash';
    case 'dropStash': return 'Drop stash';
    default: return 'Go ahead';
  }
}

/**
 * Performs `write`: `discards` asks the core what it discards, and where it discards anything,
 * `ask` puts the question naming it and resolves with the answer; `call` asks the host to write,
 * confirmed where the developer said yes. On a decline nothing is written.
 */
export async function confirmedWrite(
  write: Write,
  discards: (write: Write) => Promise<string | null>,
  ask: (lost: string, write: Write) => Promise<boolean>,
  call: (write: Write, confirmed: boolean) => Promise<void>,
): Promise<'kept' | 'written'> {
  const lost = await discards(write);
  if (lost !== null && !(await ask(lost, write))) return 'kept';
  await call(write, lost !== null);
  return 'written';
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
