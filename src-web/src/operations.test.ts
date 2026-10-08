// Verifies: ADR-0033, G-5
import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import { historyOperations, type Offered } from './operations.ts';
import type { RepoRefs } from './types.ts';

describe('the operations a commit offers', () => {
  // Written by the core's test (crates/core/tests/write_fixture.rs); this side only reads it.
  const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'operations.json');

  test('each answer is the core\'s', async () => {
    const cases = (await Bun.file(fixture).json()) as { refs: RepoRefs; sha: string; answer: Offered }[];
    expect(cases.length).toBe(4);
    for (const { refs, sha, answer } of cases) {
      expect(historyOperations(refs, sha)).toEqual(answer);
    }
  });
});
