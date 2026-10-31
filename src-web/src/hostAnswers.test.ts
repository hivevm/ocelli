// Verifies: ADR-draft-row-window-margin
import { describe, expect, test } from 'bun:test';
import { Newest } from './hostAnswers.ts';

/** A request whose answer the test hands over when it likes. */
function pending<T>(): { request: () => Promise<T>; answer: (value: T) => void; fail: (e: unknown) => void } {
  let answer!: (value: T) => void;
  let fail!: (e: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    answer = resolve;
    fail = reject;
  });
  return { request: () => promise, answer, fail };
}

describe('Newest', () => {
  test('keeps the answer to the newest request for the current load', async () => {
    const newest = new Newest();
    const asked = newest.ask(async () => ({ generation: 1, rows: 3 }), () => 1);
    expect(await asked).toEqual({ generation: 1, rows: 3 });
  });

  test('drops an answer overtaken by a newer request, even when it arrives last', async () => {
    const newest = new Newest();
    const older = pending<{ generation: number }>();
    const newer = pending<{ generation: number }>();
    const first = newest.ask(older.request, () => 1);
    const second = newest.ask(newer.request, () => 1);
    newer.answer({ generation: 1 });
    older.answer({ generation: 1 });
    expect(await second).toEqual({ generation: 1 });
    expect(await first).toBeNull();
  });

  test('drops an answer of a load replaced while it was on the way', async () => {
    const newest = new Newest();
    let generation = 1;
    const on = pending<{ generation: number }>();
    const asked = newest.ask(on.request, () => generation);
    generation = 2;
    on.answer({ generation: 1 });
    expect(await asked).toBeNull();
  });

  test('drops the answers in flight when the requests are dropped', async () => {
    const newest = new Newest();
    const on = pending<{ generation: number }>();
    const asked = newest.ask(on.request, () => 1);
    newest.drop();
    on.answer({ generation: 1 });
    expect(await asked).toBeNull();
  });

  test('a null answer stays null', async () => {
    expect(await new Newest().ask(async () => null, () => 1)).toBeNull();
  });

  test('a failed request rejects, for the caller to report', async () => {
    const newest = new Newest();
    const on = pending<{ generation: number }>();
    const asked = newest.ask(on.request, () => 1);
    on.fail('no repository open');
    await expect(asked).rejects.toBe('no repository open');
  });
});
