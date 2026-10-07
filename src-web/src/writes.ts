// The web interface's side of a write (ADR-0033): what the tag form hands the host, asking before
// a write that discards work (ADR-0034), and which commit to select again once the reload after a
// write has laid it out.
//
// Both are bookkeeping over strings and numbers, so they live here and are tested directly.

/** A tag the host is asked to create; `message` null makes a lightweight tag. */
export interface TagRequest {
  sha: string;
  name: string;
  message: string | null;
}

/** The tag the form describes, or null while it has no name. A blank message is none. */
export function tagRequest(sha: string, name: string, message: string): TagRequest | null {
  const trimmed = name.trim();
  if (trimmed === '') return null;
  return { sha, name: trimmed, message: message.trim() === '' ? null : message };
}

/** A tag as the refs read it, which deleting expects to find unchanged. */
export interface ShownTag {
  name: string;
  sha: string;
  annotated: boolean;
}

/** What deleting `tag` discards, in the words the core's `Write::discards` uses. */
export function deletionLoss(tag: ShownTag): string {
  return tag.annotated ? `the tag ${tag.name} and its message` : `the tag ${tag.name}`;
}

/**
 * Deletes `tag` once the developer confirms it: `ask` puts the question, naming what is lost, and
 * resolves with the answer; `call` asks the host. On a decline nothing is called.
 */
export async function deleteTag(
  tag: ShownTag,
  ask: (lost: string) => Promise<boolean>,
  call: (tag: ShownTag, confirmed: true) => Promise<void>,
): Promise<'kept' | 'deleted'> {
  if (!(await ask(deletionLoss(tag)))) return 'kept';
  await call(tag, true);
  return 'deleted';
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
