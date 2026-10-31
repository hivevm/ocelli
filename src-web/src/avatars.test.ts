// Verifies: ADR-0025, ADR-0035, G-18
import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { avatarsOn } from './avatars.ts';

describe('avatars', () => {
  test('are off unless the developer turned them on', () => {
    expect(avatarsOn(null)).toBe(false);
    expect(avatarsOn('off')).toBe(false);
    expect(avatarsOn('')).toBe(false);
    expect(avatarsOn('ON')).toBe(false);
    expect(avatarsOn('on')).toBe(true);
  });

  test('every image the interface draws from elsewhere stands inside the avatars switch', () => {
    // An `<img>` is the one way the interface asks another host for something; the policy grants
    // Gravatar for images alone. Each one must be drawn only while the switch is on.
    const images: string[] = [];
    for (const name of readdirSync(import.meta.dir).filter((n) => n.endsWith('.svelte'))) {
      const source = readFileSync(join(import.meta.dir, name), 'utf8');
      let at = source.indexOf('<img');
      while (at !== -1) {
        const before = source.slice(0, at);
        const opened = before.lastIndexOf('{#if showAvatars');
        const closed = before.lastIndexOf('{/if}');
        images.push(`${name}: ${opened > closed ? 'guarded' : 'unguarded'}`);
        at = source.indexOf('<img', at + 1);
      }
    }
    // The scan sees the avatars the commit detail draws.
    expect(images.length).toBeGreaterThan(0);
    expect(images.filter((i) => i.endsWith('unguarded'))).toEqual([]);
  });
});
