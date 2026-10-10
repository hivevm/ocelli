// Verifies: ADR-0039, ADR-0027
import { describe, expect, test } from 'bun:test';
import { OUTSIDE_ROOT, parentBlocked, submoduleBlocked, worktreeBlocked } from './repoSwitch.ts';

describe('repoSwitch', () => {
  test('a working tree switches unless it is on screen or lies outside the server root', () => {
    expect(worktreeBlocked({ path: '/home/a/r', isCurrent: false })).toBeUndefined();
    expect(worktreeBlocked({ path: 'inside', isCurrent: false })).toBeUndefined();
    expect(worktreeBlocked({ path: '.', isCurrent: true })).toBe('the working tree you are viewing');
    expect(worktreeBlocked({ path: '', isCurrent: false })).toBe(OUTSIDE_ROOT);
  });

  test('a submodule switches unless it was never checked out or lies outside the server root', () => {
    expect(submoduleBlocked({ workdir: 'sm', state: 'inSync' })).toBeUndefined();
    expect(submoduleBlocked({ workdir: 'sm', state: 'modified' })).toBeUndefined();
    expect(submoduleBlocked({ workdir: '', state: 'uninitialized' })).toBe(
      'not initialized — no working copy to open',
    );
    expect(submoduleBlocked({ workdir: '', state: 'inSync' })).toBe(OUTSIDE_ROOT);
    expect(submoduleBlocked({ workdir: '', state: 'modified' })).toBe(OUTSIDE_ROOT);
  });

  test('the parent row switches unless it lies outside the server root', () => {
    expect(parentBlocked({ path: '.' })).toBeUndefined();
    expect(parentBlocked({ path: '/home/a/super' })).toBeUndefined();
    expect(parentBlocked({ path: '' })).toBe(OUTSIDE_ROOT);
  });
});
