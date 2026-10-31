// Verifies: ADR-0024
//
// The theming decision (ADR-0024) rests on two properties that a reader cannot see from one
// palette: both themes carry the same token set, so no component can reach for a token that
// exists in one theme only, and the lane colours are the same in both, so a commit keeps its
// colour across a theme switch. Both are cheap to break while editing colours by hand.

import { describe, expect, it } from 'bun:test';
import { SERIES_COLORS, THEMES, seriesColor } from './palette.ts';

describe('theme palette', () => {
  it('defines exactly the two themes the decision names', () => {
    expect(Object.keys(THEMES).sort()).toEqual(['dark', 'light']);
  });

  it('carries the same token set in both themes', () => {
    const dark = Object.keys(THEMES.dark).sort();
    const light = Object.keys(THEMES.light).sort();
    expect(light).toEqual(dark);
  });

  it('gives every token a value in both themes', () => {
    for (const [name, colors] of Object.entries(THEMES)) {
      for (const [token, value] of Object.entries(colors)) {
        expect(value, `${name}.${token}`).toMatch(/^(#[0-9a-fA-F]{3,8}|rgba?\(|hsla?\()/);
      }
    }
  });

  it('keeps the lane colours out of the themes, so they cannot diverge', () => {
    // One array, no per-theme table: there is no second place where a lane colour could be
    // edited for one theme only. A token named after lanes or series in either theme would be
    // that second place. (A lane colour may still equal a token by intent - the first one is
    // the dark theme's accent.)
    for (const [name, colors] of Object.entries(THEMES)) {
      for (const token of Object.keys(colors)) {
        expect(token, `${name}.${token}`).not.toMatch(/lane|series/i);
      }
    }
    expect(SERIES_COLORS.length).toBeGreaterThan(1);
  });

  it('cycles the lane colours instead of running out', () => {
    expect(seriesColor(0)).toBe(SERIES_COLORS[0]);
    expect(seriesColor(SERIES_COLORS.length)).toBe(SERIES_COLORS[0]);
    expect(seriesColor(SERIES_COLORS.length + 2)).toBe(SERIES_COLORS[2]);
  });
});
