// Verifies: ADR-0015
import { describe, expect, test } from 'bun:test';
import { roundedPath } from './edgePath.ts';

const p = (x: number, y: number) => ({ x, y });

describe('roundedPath', () => {
  test('a straight edge is one line from the parent to the child', () => {
    expect(roundedPath([p(10, 100), p(10, 20)], 5)).toEqual([
      { kind: 'move', to: p(10, 100) },
      { kind: 'line', to: p(10, 20) },
    ]);
  });

  test('an L-shaped edge bends once, the curve starting and ending r from the corner', () => {
    // A branch edge: along the parent's row to the right, then up the child's lane.
    expect(roundedPath([p(10, 100), p(50, 100), p(50, 20)], 5)).toEqual([
      { kind: 'move', to: p(10, 100) },
      { kind: 'line', to: p(45, 100) },
      { kind: 'curve', via: p(50, 100), to: p(50, 95) },
      { kind: 'line', to: p(50, 20) },
    ]);
  });

  test('a merge to the left bends at the top', () => {
    expect(roundedPath([p(50, 100), p(50, 20), p(10, 20)], 5)).toEqual([
      { kind: 'move', to: p(50, 100) },
      { kind: 'line', to: p(50, 25) },
      { kind: 'curve', via: p(50, 20), to: p(45, 20) },
      { kind: 'line', to: p(10, 20) },
    ]);
  });

  test('a merge out of the main line bends at each end of its track', () => {
    expect(roundedPath([p(10, 100), p(70, 100), p(70, 20), p(30, 20)], 5)).toEqual([
      { kind: 'move', to: p(10, 100) },
      { kind: 'line', to: p(65, 100) },
      { kind: 'curve', via: p(70, 100), to: p(70, 95) },
      { kind: 'line', to: p(70, 25) },
      { kind: 'curve', via: p(70, 20), to: p(65, 20) },
      { kind: 'line', to: p(30, 20) },
    ]);
  });

  test('an edge down to a late child bends the other way', () => {
    expect(roundedPath([p(10, 20), p(50, 20), p(50, 100)], 5)).toEqual([
      { kind: 'move', to: p(10, 20) },
      { kind: 'line', to: p(45, 20) },
      { kind: 'curve', via: p(50, 20), to: p(50, 25) },
      { kind: 'line', to: p(50, 100) },
    ]);
  });
});
