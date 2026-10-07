// The web interface's side of a write (ADR-0033): the writes it hands the host, asking before one
// that discards work in the words the core names it with (ADR-0034), and which commit to select
// again once the reload after a write has laid it out.
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

/** The place of a stash node's entry in the stash, read from its `stash@{n}` ref (ADR-0024). */
export function stashIndex(refs: string[]): number | null {
  for (const ref of refs) {
    const m = /^(?:📦 )?stash@\{(\d+)\}$/u.exec(ref);
    if (m) return Number(m[1]);
  }
  return null;
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

/** A load's progress, as far as selecting again needs it. */
export interface Progress {
  generation: number;
  loading: boolean;
}

/**
 * The commit selected when a write was done, to select again in the load that follows it. Rows
 * of the load the write ran in are no use: the reload's own rows are looked up as they grow,
 * until the commit is found or the reload ends without it.
 */
export class Reselect {
  #sha: string | null = null;
  #after = 0;

  /** Remembers `sha`, to look up once a load newer than `generation` grows. */
  remember(sha: string, generation: number): void {
    this.#sha = sha;
    this.#after = generation;
  }

  /** The commit to look up in the rows `progress` reports, or null. */
  wanted(progress: Progress): string | null {
    return progress.generation > this.#after ? this.#sha : null;
  }

  /**
   * Takes the lookup's answer for `sha`: whether to select it now. A commit not laid out yet
   * stays wanted while the load runs, and is given up once it has ended.
   */
  settle(sha: string, laidOut: boolean, progress: Progress): boolean {
    if (this.#sha !== sha) return false;
    if (laidOut || !progress.loading) this.#sha = null;
    return laidOut;
  }

  /** Gives the commit up, as when the developer selects another. */
  forget(): void {
    this.#sha = null;
  }
}
