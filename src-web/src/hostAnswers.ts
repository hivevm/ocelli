// Which of the host's answers the web interface keeps (ADR-0031).
//
// The host answers commands asynchronously while the history grows and while another repository
// may be opened, so an answer can arrive after a newer request of the same kind, or after the
// load it belongs to was replaced. Keeping only the newest answer for the current load is plain
// bookkeeping over numbers, so it lives here and is tested directly.

/** An answer that says which load it belongs to. */
export interface Answer {
  generation: number;
}

/** Keeps the newest answer of one kind of request for the current load. */
export class Newest {
  #asked = 0;

  /**
   * Asks with `request` and resolves with its answer, or with null when a newer request was made
   * or the requests were dropped meanwhile, when the answer belongs to another load than
   * `generation` names once it arrives, or when the host answers null. A failed request rejects,
   * for the caller to report.
   */
  async ask<T extends Answer>(
    request: () => Promise<T | null>,
    generation: () => number | undefined,
  ): Promise<T | null> {
    const asked = ++this.#asked;
    const answer = await request();
    if (asked !== this.#asked || answer === null || answer.generation !== generation()) return null;
    return answer;
  }

  /** Drops every request in flight: their answers resolve null. */
  drop(): void {
    this.#asked++;
  }
}
