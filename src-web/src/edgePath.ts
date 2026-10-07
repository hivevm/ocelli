// How the canvas draws an edge through its corners (ADR-0030).
//
// The core decides which corners an edge runs through, from its parent to its child; the canvas
// only turns them into a path with each inner corner softened by a short quadratic curve. That
// is plain geometry over points, so it lives here and is tested directly.

/** A point on the canvas. */
export interface Point {
  x: number;
  y: number;
}

/** One drawing step of a path. */
export type Step =
  | { kind: 'move'; to: Point }
  | { kind: 'line'; to: Point }
  | { kind: 'curve'; via: Point; to: Point };

/** The unit step from `a` towards `b`, along the one axis they differ on. */
function towards(a: Point, b: Point): Point {
  return { x: Math.sign(b.x - a.x), y: Math.sign(b.y - a.y) };
}

/**
 * The steps that draw a path through `corners`, the first the parent and the last the child:
 * straight lines between them, each inner corner rounded by a quadratic curve that starts `r`
 * before it and ends `r` after it, with its control point on the corner.
 */
export function roundedPath(corners: Point[], r: number): Step[] {
  if (corners.length === 0) return [];
  const steps: Step[] = [{ kind: 'move', to: corners[0] }];
  for (let i = 1; i < corners.length - 1; i++) {
    const [before, corner, after] = [corners[i - 1], corners[i], corners[i + 1]];
    const into = towards(before, corner);
    const out = towards(corner, after);
    steps.push({ kind: 'line', to: { x: corner.x - into.x * r, y: corner.y - into.y * r } });
    steps.push({ kind: 'curve', via: corner, to: { x: corner.x + out.x * r, y: corner.y + out.y * r } });
  }
  if (corners.length > 1) steps.push({ kind: 'line', to: corners[corners.length - 1] });
  return steps;
}
