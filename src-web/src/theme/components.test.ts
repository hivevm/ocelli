// Verifies: ADR-0021
//
// Every component style takes its colours from the theme's tokens (ADR-0021), so a theme switch
// reaches it. A colour literal in a `<style>` block, or a `var()` naming no token, is a colour
// the switch never reaches; the palettes' own tests cannot see either. Markup, such as an inline
// icon's `fill`, and the canvas palettes are outside this test.

import { describe, expect, it } from 'bun:test';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const SRC = join(import.meta.dir, '..');

/** The CSS named colours; `transparent` and `currentColor` name no colour of their own. */
const NAMED = new Set(
  ('aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue ' +
    'blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk ' +
    'crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki ' +
    'darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen ' +
    'darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue ' +
    'dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite ' +
    'gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki ' +
    'lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan ' +
    'lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen ' +
    'lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen ' +
    'magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen ' +
    'mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream ' +
    'mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid ' +
    'palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum ' +
    'powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown ' +
    'seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen ' +
    'steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow ' +
    'yellowgreen').split(' '),
);

/** The custom properties `css` declares. */
function declared(css: string): Set<string> {
  return new Set([...css.matchAll(/(--[a-z0-9-]+)\s*:/gi)].map((m) => m[1]));
}

/** What is wrong with the `<style>` blocks of a component, as `line: problem`. */
export function problems(source: string, tokens: Set<string>): string[] {
  const found: string[] = [];
  for (const block of source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
    const start = block.index + block[0].indexOf(block[1]);
    const css = block[1].replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
    const local = declared(css);
    for (const decl of css.matchAll(/([a-z-]+)\s*:\s*([^;{}]+)(?=[;}])/gi)) {
      const value = decl[2];
      const line = source.slice(0, start + decl.index).split('\n').length;
      // A token's name may hold a colour's, as `--blue` does.
      const bare = value.replace(/var\(--[a-z0-9-]+/gi, 'var(');
      const literal =
        /#[0-9a-f]{3,8}\b/i.exec(bare) ??
        /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/i.exec(bare) ??
        bare.split(/[^a-z]+/i).find((word) => NAMED.has(word.toLowerCase()));
      if (literal) found.push(`${line}: colour literal in ${decl[1]}: ${value.trim()}`);
      for (const v of value.matchAll(/var\((--[a-z0-9-]+)/gi)) {
        if (!tokens.has(v[1]) && !local.has(v[1])) found.push(`${line}: ${v[1]} is no token`);
      }
    }
  }
  return found;
}

describe('component styles', () => {
  const tokens = declared(readFileSync(join(SRC, 'theme', 'theme.css'), 'utf8'));
  const components = readdirSync(SRC).filter((f) => f.endsWith('.svelte'));

  it('take every colour from a token the theme defines', () => {
    expect(components.length).toBeGreaterThan(0);
    for (const file of components) {
      expect(problems(readFileSync(join(SRC, file), 'utf8'), tokens), file).toEqual([]);
    }
  });

  it('are refused with a literal or a token the theme lacks, by line', () => {
    const source = [
      '<div></div>',
      '<style>',
      '  .a { color: var(--text); background: transparent; }',
      '  .b { border: 1px solid #fff; }',
      '  .c { box-shadow: 0 0 4px rgb(0 0 0 / 40%); }',
      '  .d { color: white; }',
      '  .e { color: var(--nowhere); }',
      '  .f { --local: var(--text); color: var(--local); }',
      '  .g:hover { color: currentColor; }',
      '</style>',
    ].join('\n');
    expect(problems(source, new Set(['--text']))).toEqual([
      '4: colour literal in border: 1px solid #fff',
      '5: colour literal in box-shadow: 0 0 4px rgb(0 0 0 / 40%)',
      '6: colour literal in color: white',
      '7: --nowhere is no token',
    ]);
  });
});
