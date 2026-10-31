# Theme

A portable dark/light theme: one CSS file of design tokens, one TypeScript file
that mirrors the colors for imperative renderers, and the two helpers that switch
themes. Nothing here imports from the rest of the application, so the directory
can be copied into another project as it stands.

The design behind it — why tokens *and* a JS palette — is
[ADR-0024](../../../docs/adr/0024-themes-tokens-and-palettes.md).

| File         | Contents                                                              |
| ------------ | --------------------------------------------------------------------- |
| `theme.css`  | Reset, all tokens (dark + light overrides), base `body` typography     |
| `palette.ts` | The same colors as data (`THEMES`), font stacks, series colors, helpers |

## Setup

**1. Import the stylesheet once**, at the entry point, before the components:

```ts
import './theme/theme.css';
```

**2. Apply the stored theme before first paint** so a light-mode start does not
flash dark. This has to run inline in `index.html`, ahead of the bundle:

```html
<script>
  try {
    document.documentElement.dataset.theme =
      localStorage.getItem('theme') === 'light' ? 'light' : 'dark';
  } catch (e) { /* ignore */ }
</script>
```

**3. Drive the toggle from `palette.ts`** — `storedTheme()` reads the persisted
choice, `applyTheme()` sets `data-theme` on `<html>` and persists it:

```ts
import { applyTheme, storedTheme, type Theme } from './theme/palette.ts';

let theme: Theme = storedTheme();
applyTheme(theme);                       // on start and after every change
```

Dark needs no attribute; light is `<html data-theme="light">`.

## Token vocabulary

Reference tokens as `var(--token)`; never write a hex literal in a component.

**Surfaces** — `--bg-app` (the window's canvas), `--bg-chrome` (title/status bars),
`--bg-panel` (sidebars, recessed lists), `--bg-elev` (raised controls: buttons,
inputs), `--bg-hover`, `--bg-sel` (selected row).

**Lines and elevation** — `--border` (default hairline), `--border-strong` (a
control's own outline), `--border-hover`, `--btn-hover-bg`, `--shadow` (a color
with alpha, for `box-shadow`).

**Text**, brightest to faintest — `--text-brightest` (headings, the selected row),
`--text-bright` (hover), `--text` (body, set on `body`), `--text-2` (secondary
lines), `--text-muted` (labels), `--text-dim` (metadata), `--text-dimmer`,
`--text-faint` (empty-state watermarks). Picking a level is picking an importance,
so the ladder holds in both themes: in light mode the values invert, the roles
do not.

**Accent** — the one brand color. `--accent` for text and icons, `--accent-bright`
for its hover, and two tinted surfaces: `--accent-bg` / `--accent-border` for
quiet badges, `--accent-bg-strong` / `--accent-border-strong` (plus their
`-hover` variants) for the primary button.

**Status and categories** — `--blue`, `--amber`, `--red`, `--teal`, each with a
`-bg` tint (and `-border` where a chip needs one), plus `--error` for failures.
Use the color for the foreground and its `-bg` behind it; the pair is contrast-
checked in both themes, mixing across pairs is not.

**Metrics** (identical in both themes) — `--font-ui`, `--font-mono`,
`--font-size` (the 13px base), `--radius-xs` (3px, chips and badges),
`--radius-sm` (4px), `--radius-md` (5px, buttons and inputs), `--radius-lg`
(8px, cards), and `--transition` for hover/state animations.

**Diff view** (optional — delete both blocks when the project has no patch view)
— `--diff-gutter-bg`, and per side `--diff-add-*` / `--diff-del-*`
(`-bg`, `-gutter`, `-border`, `-text`) plus `--diff-hunk-*` for hunk headers.

## Rules

- **The CSS file and `palette.ts` are one palette in two forms.** A color that
  changes changes in both; `THEMES` deliberately carries only the tokens an
  imperative renderer needs, not the CSS-only ones.
- **Reach for an existing token before adding one.** New tokens are added to
  both `:root` and the light block in the same edit — a token defined in only
  one theme is a bug that shows up as an unreadable light mode.
- **Recolor by editing the values, not the names.** The token names describe
  roles, so a project restyles itself by replacing the hex values (and the
  matching entries in `palette.ts`); component code is untouched.
- `SERIES_COLORS` / `seriesColor(i)` is the categorical set for lanes, chart
  series, or avatars — items that only need to be told apart. It is the same in
  both themes on purpose, so an item keeps its color across a switch.
