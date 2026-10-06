<script lang="ts">
  import type { Found } from './types.ts';

  interface Props {
    /** Searches the history the host keeps (G-16). */
    search: (query: string) => Promise<Found>;
    /** Changes as the history grows, so the matches cover the rows that arrive. */
    revision: number;
    onJump: (row: number) => void;
  }
  let { search, revision, onJump }: Props = $props();

  let query = $state('');
  let input = $state<HTMLInputElement | null>(null);
  // Position of the last match jumped to; -1 = not navigating yet. Reset
  // whenever the query (and thus the match list) changes; rows arriving as the
  // history loads lie below every match, so they keep it.
  let pos = $state(-1);
  let found = $state<Found>({ generation: 0, rows: [], hits: [] });
  const matches = $derived(found.rows);
  /** Numbers the searches, so an answer overtaken by a newer one is dropped. */
  let searches = 0;

  $effect(() => {
    const q = query;
    void revision;
    const n = ++searches;
    if (q.trim() === '') {
      found = { generation: 0, rows: [], hits: [] };
      return;
    }
    void search(q).then((f) => {
      if (n !== searches) return;
      // Another load's rows are other commits: the position starts over.
      if (f.generation !== found.generation) pos = -1;
      found = f;
    });
  });

  // Results panel: open while the field has focus and a query is set. Rows
  // use onmousedown-preventDefault so clicking them does not blur the input.
  let focused = $state(false);
  let panel = $state<HTMLElement | null>(null);
  const panelOpen = $derived(focused && query.trim() !== '' && matches.length > 0);
  // Rendering thousands of rows would make typing sluggish; everything beyond
  // the cap is reachable via Enter-cycling and shown as a "+N more" footer. The
  // host sends the nodes of as many matches (`SEARCH_HITS`), the rows of all.
  const MAX_PANEL_ROWS = 200;

  $effect(() => {
    void query;
    pos = -1;
  });

  // Keep the active row visible while Enter/Shift+Enter cycle through it.
  $effect(() => {
    void pos;
    panel?.querySelector('.active')?.scrollIntoView({ block: 'nearest' });
  });

  /** Focus + select the field — bound to Ctrl/Cmd+F in App.svelte. */
  export function focusSearch(): void {
    input?.focus();
    input?.select();
  }

  /** Enter jumps to the next match, Shift+Enter to the previous; wraps around. */
  function jumpToMatch(direction: 1 | -1): void {
    const len = matches.length;
    if (len === 0) return;
    pos = pos === -1 && direction === -1 ? len - 1 : (pos + direction + len) % len;
    onJump(matches[pos]);
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      jumpToMatch(e.shiftKey ? -1 : 1);
    } else if (e.key === 'Escape') {
      query = '';
      (e.currentTarget as HTMLInputElement).blur();
    }
  }

  function pickMatch(index: number): void {
    pos = index;
    onJump(matches[index]);
  }
</script>

<div
  id="search-box"
  class:no-hits={query.trim() !== '' && matches.length === 0}
  onfocusin={() => (focused = true)}
  onfocusout={() => (focused = false)}
>
  <svg class="search-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
    <circle cx="7" cy="7" r="4.5" />
    <line x1="10.5" y1="10.5" x2="14" y2="14" stroke-linecap="round" />
  </svg>
  <input
    bind:this={input}
    bind:value={query}
    onkeydown={onKeydown}
    placeholder="Search history (Ctrl+F)"
    spellcheck="false"
  />
  {#if query.trim() !== ''}
    <span class="search-count">
      {pos === -1 ? matches.length : `${pos + 1}/${matches.length}`}
    </span>
    <button
      class="search-nav"
      title="Previous match (Shift+Enter)"
      aria-label="Previous match"
      disabled={matches.length === 0}
      onclick={() => jumpToMatch(-1)}
    >
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M4 10l4-4 4 4" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
    <button
      class="search-nav"
      title="Next match (Enter)"
      aria-label="Next match"
      disabled={matches.length === 0}
      onclick={() => jumpToMatch(1)}
    >
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M4 6l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  {/if}

  {#if panelOpen}
    <div class="search-panel" bind:this={panel}>
      {#each found.hits.slice(0, MAX_PANEL_ROWS) as match, i (match.sha)}
        <button
          class="search-hit"
          class:active={i === pos}
          onmousedown={(e) => e.preventDefault()}
          onclick={() => pickMatch(i)}
        >
          <span class="hit-sha">{match.kind === 'working' ? 'WIP' : match.shortSha}</span>
          <span class="hit-message" title={match.message}>{match.message}</span>
          <span class="hit-author">{match.author}</span>
        </button>
      {/each}
      {#if matches.length > MAX_PANEL_ROWS}
        <div class="search-more">
          +{matches.length - MAX_PANEL_ROWS} more — cycle with Enter or refine the query
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  #search-box {
    position: relative; /* anchors the results panel */
    display: flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 6px 0 8px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--text-muted);
  }

  #search-box:focus-within {
    border-color: var(--accent-border-strong);
  }

  #search-box.no-hits:focus-within {
    border-color: var(--red);
  }

  #search-box .search-icon {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  #search-box input {
    width: 170px;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text);
    font-size: 12px;
  }

  #search-box input::placeholder {
    color: var(--text-dim);
  }

  .search-count {
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .no-hits .search-count {
    color: var(--red);
  }

  .search-nav {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--radius-xs);
    color: var(--text-muted);
    cursor: pointer;
  }

  .search-nav svg {
    width: 12px;
    height: 12px;
  }

  .search-nav:hover:not(:disabled) {
    background: var(--btn-hover-bg);
    color: var(--text-bright);
  }

  .search-nav:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .search-panel {
    position: absolute;
    top: 30px;
    right: -1px; /* align with the box border */
    width: 380px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    box-shadow: 0 8px 24px var(--shadow);
    z-index: 200; /* above the diff overlay */
    display: flex;
    flex-direction: column;
    padding: 4px 0;
  }

  .search-hit {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    background: transparent;
    border: none;
    text-align: left;
    cursor: pointer;
    min-width: 0;
    color: inherit;
    font-family: inherit;
  }

  .search-hit:hover {
    background: var(--bg-hover);
  }

  .search-hit.active {
    background: var(--bg-sel);
  }

  .hit-sha {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--blue);
    width: 52px;
    flex-shrink: 0;
  }

  .hit-message {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .hit-author {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-more {
    padding: 5px 10px 3px;
    font-size: 10px;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
    margin-top: 3px;
  }
</style>
