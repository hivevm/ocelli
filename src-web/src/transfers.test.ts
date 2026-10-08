// Verifies: ADR-0035, G-4
import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import { canStop, fetchOf, pullOf, pushOf } from './transfers.ts';
import type { RepoRefs } from './types.ts';
import type { Write } from './writes.ts';

describe('the transfers of the checked-out branch', () => {
  // Written by the core's test (crates/core/tests/write_fixture.rs): its answers over a few sets
  // of refs, and for every write whether it can be stopped; this side only reads it.
  const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'transfers.json');

  test('each answer is the core\'s', async () => {
    const { cases } = (await Bun.file(fixture).json()) as {
      cases: { refs: RepoRefs; fetch: unknown; pull: unknown; push: unknown }[];
    };
    expect(cases.length).toBeGreaterThan(3);
    for (const { refs, fetch, pull, push } of cases) {
      expect(fetchOf(refs)).toEqual(fetch as ReturnType<typeof fetchOf>);
      expect(pullOf(refs)).toEqual(pull as ReturnType<typeof pullOf>);
      expect(pushOf(refs)).toEqual(push as ReturnType<typeof pushOf>);
    }
  });

  test('a write can be stopped where the core says so', async () => {
    const { canStop: expected } = (await Bun.file(fixture).json()) as {
      canStop: Record<Write['kind'], boolean>;
    };
    for (const [kind, stops] of Object.entries(expected)) {
      expect([kind, canStop({ kind } as Write)]).toEqual([kind, stops]);
    }
  });
});
