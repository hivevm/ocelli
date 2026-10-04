# ADR-0022: The web interface is built with Svelte 5, and the commit graph stays imperative canvas drawing

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** the component framework and the build of `src-web/`

## Context

[ADR-0008](0008-toolchain-rust-and-typescript.md) writes the one web interface, shared by the
desktop application and the remote server, in TypeScript on Bun, and leaves its framework open.
Other decisions already need what only a component model provides:
[ADR-0020](0020-diff-overlay-and-file-list.md) draws the file tree recursively and holds the view
mode in state, and [ADR-0021](0021-themes-tokens-and-palettes.md) selects the canvas palette from a
`theme` prop that is part of the drawing effect's dependencies.

The requirement is narrow, and it rules most candidates in or out. The main content, the commit
graph, is drawn imperatively on a `<canvas>` and must not be re-rendered by a framework at all;
the frame budget belongs to the canvas ([ADR-0017](0017-virtual-rendering.md)). What needs a
component model is the chrome around it: toolbar, commit detail, the changed files in two views,
the diff, the repository sidebar, the status bar, and the theme toggle. That is ordinary stateful
DOM, with one recursive view (the directory tree) and one place where a change of state must
trigger an imperative redraw (the theme switch). The remote server sends the same bundle to the
browser, so its size counts on every load.

## Decision

We will build the web interface as **Svelte 5 single-file components** with `<script lang="ts">`,
compiled by `@sveltejs/vite-plugin-svelte` in a Vite build and reactive through Svelte 5's runes
(`$state`, `$derived`, `$effect`, `$props`), with Svelte owning the DOM chrome and never the
commit graph, which stays imperative canvas drawing that Svelte passes props to and re-runs one
effect for when they change.

**Out of scope:** how the web interface reaches the desktop host or the remote server, which for
the remote server is [ADR-0037](0037-remote-server-websocket-per-tab.md); the drawing of the
commit graph itself.

## Alternatives considered

- **Plain TypeScript and Vite, no framework** — no dependency, but the chrome is stateful and
  grows: the recursive tree and the coordinated theme switch would be hand-written DOM
  construction and hand-tracked invalidation, precisely the code a compiler generates more
  reliably.
- **React** — the largest ecosystem, but its virtual DOM is overhead for an interface whose main
  surface is a canvas drawn by hand, and it brings a runtime into a bundle where the commit graph,
  not the chrome, should own the budget.
- **Solid** — the closest on the merits: fine-grained reactivity, no virtual DOM, a model very
  like runes. Not chosen for its smaller ecosystem and because JSX adds a markup layer that
  Svelte's single-file components cover directly, with scoped styles that suit ADR-0021's tokens.
- **Vue** — a mature single-file-component model, but a larger runtime than Svelte's compiled
  output and no advantage for an interface centred on a canvas.
- **Lit or native web components** — standards-based with a small runtime, but no compile-time
  reactivity, and templates markedly more verbose for the tree and the diff.

## Sources / Prior art

- Svelte, the compiler approach without a virtual DOM, and Svelte 5 runes — <https://svelte.dev/>
- `@sveltejs/vite-plugin-svelte` — <https://github.com/sveltejs/vite-plugin-svelte>
- Solid — <https://www.solidjs.com/>; React — <https://react.dev/>; Vue — <https://vuejs.org/>;
  Lit — <https://lit.dev/>
- Tauri, *Frontend configuration*, which lists Svelte among the supported setups —
  <https://v2.tauri.app/start/frontend/>
- GitGraph's `docs/adr/0017-svelte-frontend-framework.md` on `main`, where this interface runs.

## Consequences

- Positive: ADR-0020's recursive tree and ADR-0021's redraw on a theme switch are a few lines each;
  the compiled output ships no virtual-DOM runtime, so the frame budget stays with the canvas and
  the bundle the remote server sends stays small; single-file components keep each panel's markup,
  styles, and logic together, and scoped styles compose with ADR-0021's custom properties;
  `lang="ts"` extends the type check into the views.
- Negative / trade-offs: a framework dependency and a compile step; runes are specific to Svelte 5,
  so a future major version is a migration rather than an upgrade; a smaller ecosystem than
  React's; two models of interaction side by side, declarative chrome and an imperative canvas,
  which contributors keep apart.
- Follow-ups: none.

## Enforcement

**Not mechanically decidable:** that Svelte 5 is the right framework. A test can see that
`src-web/package.json` names it, which restates the decision instead of deciding it. That Svelte
does not own the commit graph is review: `GraphCanvas.svelte` draws the rows in view imperatively
on a canvas, and no Svelte component renders a commit or an edge.
