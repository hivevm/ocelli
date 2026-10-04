/**
 * The color tokens of `theme.css` as plain data, for renderers that cannot read
 * CSS custom properties cheaply — a `<canvas>` repainting per frame, an SVG
 * generator, a chart library (ADR-0021).
 *
 * This file and `theme.css` are two views of one palette and are kept in sync by
 * hand: when a color changes, change it in both. Only colors that imperative
 * renderers actually need live here; CSS-only tokens (the diff view) do not.
 */

/** The two themes. `dark` is the default; `light` is opted into per document. */
export type Theme = 'dark' | 'light';

/** Color tokens, named after their `--kebab-case` counterparts in `theme.css`. */
export interface ThemeColors {
  bgApp: string;
  bgChrome: string;
  bgPanel: string;
  bgHover: string;
  bgElev: string;
  bgSel: string;

  border: string;
  borderStrong: string;
  borderHover: string;

  textBrightest: string;
  textBright: string;
  text: string;
  text2: string;
  textMuted: string;
  textDim: string;
  textDimmer: string;
  textFaint: string;

  accent: string;
  accentBright: string;
  accentBg: string;
  accentBorder: string;
  accentBgStrong: string;
  accentBorderStrong: string;

  blue: string;  blueBg: string;  blueBorder: string;
  amber: string; amberBg: string; amberBorder: string;
  red: string;   redBg: string;
  teal: string;  tealBg: string;
  error: string;
}

export const THEMES: Record<Theme, ThemeColors> = {
  dark: {
    bgApp: '#1a1d23',
    bgChrome: '#13151a',
    bgPanel: '#0f1117',
    bgHover: '#1e2128',
    bgElev: '#2a2d35',
    bgSel: '#1a2535',

    border: '#2a2d35',
    borderStrong: '#383b45',
    borderHover: '#4a4d58',

    textBrightest: '#e8eaf0',
    textBright: '#e0e3e9',
    text: '#d0d3d9',
    text2: '#c0c3ca',
    textMuted: '#8b8fa8',
    textDim: '#5a5e6e',
    textDimmer: '#4a4e5e',
    textFaint: '#3a3e4e',

    accent: '#4caf7d',
    accentBright: '#5cc48d',
    accentBg: '#1a3a22',
    accentBorder: '#2a5a34',
    accentBgStrong: '#1e4d32',
    accentBorderStrong: '#2d6e47',

    blue: '#6b9fff',  blueBg: '#162338',  blueBorder: '#1f3a5a',
    amber: '#e8a94a', amberBg: '#2e2412', amberBorder: '#4a3a1a',
    red: '#e06b75',   redBg: '#3a1a1a',
    teal: '#4ec9c9',  tealBg: '#1a2a3a',
    error: '#e05a5a',
  },
  light: {
    bgApp: '#ffffff',
    bgChrome: '#f3f4f6',
    bgPanel: '#ffffff',
    bgHover: '#eceef2',
    bgElev: '#e6e8ec',
    bgSel: '#e3edff',

    border: '#d8dbe0',
    borderStrong: '#cbd0d8',
    borderHover: '#b5bcc7',

    textBrightest: '#14171c',
    textBright: '#24272e',
    text: '#2e333d',
    text2: '#3a3f4a',
    textMuted: '#5d6470',
    textDim: '#767c8a',
    textDimmer: '#969cab',
    textFaint: '#b3b9c4',

    accent: '#2e9e63',
    accentBright: '#258a55',
    accentBg: '#e3f6ea',
    accentBorder: '#b7e4c8',
    accentBgStrong: '#d8f0e0',
    accentBorderStrong: '#aadcbf',

    blue: '#2d6fdb',  blueBg: '#e4edfb',  blueBorder: '#bcd4f5',
    amber: '#b5791f', amberBg: '#fbf0d8', amberBorder: '#ecd6a6',
    red: '#d23f4a',   redBg: '#fbe4e6',
    teal: '#1f9b9b',  tealBg: '#def2f2',
    error: '#d23f4a',
  },
};

/**
 * The font stacks of `theme.css`, for canvas contexts and anywhere else that
 * needs a font string rather than a CSS custom property.
 */
export const FONTS = {
  ui: "'Segoe UI', -apple-system, BlinkMacSystemFont, system-ui, sans-serif",
  mono: "'Cascadia Code', 'Fira Code', 'JetBrains Mono', monospace",
};

/**
 * Categorical series colors, for lanes, chart series, or any set of items that
 * only need to be told apart. Deliberately identical in both themes so an item
 * keeps its color across a theme switch; cycle them with `seriesColor(i)`.
 */
export const SERIES_COLORS = [
  '#4caf7d', // green (matches --accent in dark)
  '#6b9fff', // blue
  '#e8a94a', // amber
  '#e06b75', // red/pink
  '#b48efe', // purple
  '#4ec9c9', // teal
  '#f0a070', // peach
  '#60c0e0', // sky
  '#a0d060', // lime
  '#e080b0', // rose
];

/** The series color for index `i`, wrapping around the palette. */
export function seriesColor(i: number): string {
  return SERIES_COLORS[i % SERIES_COLORS.length];
}

/**
 * Reads the theme persisted by {@link applyTheme}; `dark` when nothing is
 * stored or storage is unavailable.
 */
export function storedTheme(): Theme {
  try {
    return localStorage.getItem('theme') === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

/** Activates `theme` on the document and persists it for the next start. */
export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem('theme', theme);
  } catch {
    /* private mode or storage disabled — the theme still applies for this session */
  }
}
