// Verifies: ADR-0037
import { describe, expect, test } from 'bun:test';
import { parentFolder } from './folders.ts';

describe('parentFolder', () => {
  test('leads up one folder at a time and stops at the server root', () => {
    expect(parentFolder('a/b/c')).toBe('a/b');
    expect(parentFolder('a/b')).toBe('a');
    expect(parentFolder('a')).toBe('.');
    expect(parentFolder('a/')).toBe('.');
    expect(parentFolder('.')).toBeNull();
    expect(parentFolder('')).toBeNull();
  });
});
