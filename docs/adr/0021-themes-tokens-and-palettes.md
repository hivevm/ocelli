# ADR-0021: Dark and light themes come from CSS variables, a canvas palette, and a terminal palette

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `src-web/src/theme/`, the colours of every Svelte component, and the canvas and terminal palettes

## Context

Ocelli offers a dark and a light theme, and colours need one source of truth so that adding or
changing a theme is one change. But Ocelli draws in three different ways: the web interface's
chrome in the DOM, styled by CSS; the commit graph imperatively on a `<canvas>`, which cannot
cheaply read CSS custom properties on every frame; and the terminal application on a grid of
cells, with neither CSS nor a canvas. The same commit should keep its lane colour in every front
end, so that a developer who learns Ocelli in one place recognises it in the others (Strategy).

## Decision

We will drive the themes through three coordinated mechanisms, switched together by one `theme`
state (`'dark' | 'light'`) toggled from the status bar: CSS custom properties for the DOM, a palette
object for the canvas, and a palette value for the terminal, all sharing one list of lane colours.

1. **DOM: CSS custom properties.** A token set is defined on `:root`, dark by default, and
   overridden under `:root[data-theme='light']`. Every component style references `var(--token)`.
   The active theme is written to `document.documentElement.dataset.theme` and kept in
   `localStorage`; an inline script in `index.html` applies the stored theme before the first
   paint, so it does not flash.
2. **Canvas: palette objects.** The drawing of the commit graph keeps two explicit palettes,
   `PALETTES.dark` and `PALETTES.light`, and selects one from a `theme` prop that is part of the
   drawing effect's dependencies, so the commit graph is redrawn on a switch. Lane colours are the
   same in both themes.
3. **Terminal: one palette value per theme.** The terminal application carries two `Palette`
   values and selects one from the same theme state. Every colour in them is an explicit RGB
   triple, the background included, so a theme looks the same whatever colours the terminal is
   configured with. The lane colours are the web interface's `SERIES_COLORS`, in the same order and
   read from the file that declares them, so a commit keeps its colour across a theme switch and
   across front ends. The choice is kept in the state file of
   [ADR-0029](0029-cli-state-file.md).

**Out of scope:** following the operating system's setting by default; a third theme.

## Alternatives considered

- **Reading CSS variables from the canvas through `getComputedStyle`** — one source of truth, but
  a style resolution per read; caching it brings back a second copy, only implicitly.
- **Following `prefers-color-scheme` only** — no toggle to build, but developers often want Ocelli
  to differ from their system's theme, and the setting could not be overridden.
- **A CSS-only theme with the canvas left dark** — least work, but the commit graph is the main
  content, and a dark graph inside a light interface looks broken.

## Sources / Prior art

- MDN, *Using CSS custom properties* —
  <https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_cascading_variables/Using_CSS_custom_properties>
- MDN, `prefers-color-scheme` —
  <https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-color-scheme>
- Catppuccin, whose Mocha palette is the dark default — <https://github.com/catppuccin/catppuccin>
- GitGraph's `docs/adr/0016-theming-css-variables-and-canvas-palette.md` on `main`, where these
  themes run.

## Consequences

- Positive: one token vocabulary drives all of the DOM, and the light theme is one override block;
  canvas, DOM, and terminal look alike in each theme; the theme survives a restart without a flash.
- Negative / trade-offs: colours are defined more than once, as CSS variables, as canvas palettes,
  and as terminal palettes, and only the lane colours are shared mechanically; a third theme
  touches the `:root` overrides, the canvas palettes, and the terminal palettes.
- Follow-ups: following the operating system's setting by default.

## Enforcement

The tests of `src-web/src/theme/palette.test.ts` cite this ADR with a `Verifies:` marker: both
themes define the same token set, and the lane colours sit outside them, so they cannot take a
per-theme value. The tests of `crates/tui/src/theme.rs` cite it for the terminal: its lane table is
compared against the web interface's `SERIES_COLORS` by reading the file that declares them, so
editing either list alone fails; every colour of both palettes is an explicit RGB triple; and the
two palettes differ. `crates/tui/src/ui.rs` cites it for the light theme reaching the screen.

`src-web/src/theme/components.test.ts` cites it for every component style taking its colours from
the tokens: no style of a Svelte component holds a colour literal.
