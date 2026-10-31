// Verifies: ADR-draft-row-window-margin, ADR-0020, G-7
import { describe, expect, test } from 'bun:test';
import { windowToAsk } from './rowWindow.ts';

describe('windowToAsk', () => {
  test('asks for the view and a margin on either side', () => {
    expect(windowToAsk(null, 100, 129, 1000, 30)).toEqual({ top: 70, rows: 90 });
  });

  test('the margin stops at the first row', () => {
    expect(windowToAsk(null, 0, 29, 1000, 30)).toEqual({ top: 0, rows: 60 });
  });

  test('scrolling within the rows asked for asks for nothing', () => {
    const asked = { top: 70, rows: 90 };
    expect(windowToAsk(asked, 70, 99, 1000, 30)).toBeNull();
    expect(windowToAsk(asked, 130, 159, 1000, 30)).toBeNull();
  });

  test('leaving the rows asked for asks again around the view', () => {
    const asked = { top: 70, rows: 90 };
    expect(windowToAsk(asked, 131, 160, 1000, 30)).toEqual({ top: 101, rows: 90 });
    expect(windowToAsk(asked, 60, 89, 1000, 30)).toEqual({ top: 30, rows: 90 });
  });

  test('the end of the history needs no covering beyond itself', () => {
    expect(windowToAsk({ top: 70, rows: 90 }, 140, 169, 150, 30)).toBeNull();
  });

  test('rows arriving below the rows asked for are asked for once in view', () => {
    // The window ended at the history's end; a chunk added rows below it.
    expect(windowToAsk({ top: 70, rows: 80 }, 140, 169, 3150, 30)).toEqual({ top: 110, rows: 90 });
  });

  test('an empty history asks for nothing', () => {
    expect(windowToAsk(null, 0, 29, 0, 30)).toBeNull();
  });
});
