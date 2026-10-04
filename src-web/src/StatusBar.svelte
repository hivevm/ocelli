<script module lang="ts">
  /** Initial status in App.svelte and the idle test for the dot below —
      shared so the two cannot drift apart. */
  export const READY_STATUS = 'Ready — open a repository to start';
</script>

<script lang="ts">
  interface Props {
    status: string;
    theme: 'dark' | 'light';
    showAvatars: boolean;
    onToggleTheme: () => void;
    onToggleAvatars: () => void;
  }
  let { status, theme, showAvatars, onToggleTheme, onToggleAvatars }: Props = $props();
</script>

<div id="statusbar">
  <div class="status-left">
    <span class="status-dot {status.startsWith('Error') ? 'error' : status === READY_STATUS ? 'idle' : 'active'}"></span>
    <span id="status">{status}</span>
  </div>
  <div class="status-right">
    <span class="status-hint">Scroll to navigate · Click or ↑/↓ to select · Esc to deselect</span>
    <button
      class="theme-toggle avatar-toggle"
      class:on={showAvatars}
      onclick={onToggleAvatars}
      title={showAvatars
        ? 'Avatars on — loaded from gravatar.com. Click to stop.'
        : 'Avatars off — click to load them from gravatar.com'}
      aria-label="Toggle loading avatars from Gravatar"
      aria-pressed={showAvatars}
    >
      <!-- person -->
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3">
        <circle cx="8" cy="5" r="2.6" />
        <path d="M2.8 13.6a5.3 5.3 0 0 1 10.4 0" stroke-linecap="round" />
      </svg>
    </button>
    <button
      class="theme-toggle"
      onclick={onToggleTheme}
      title={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
      aria-label="Toggle color theme"
    >
      {#if theme === 'dark'}
        <!-- sun -->
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3">
          <circle cx="8" cy="8" r="3.2" />
          <path d="M8 1v1.6M8 13.4V15M1 8h1.6M13.4 8H15M3 3l1.1 1.1M11.9 11.9L13 13M13 3l-1.1 1.1M4.1 11.9L3 13" stroke-linecap="round" />
        </svg>
      {:else}
        <!-- moon -->
        <svg viewBox="0 0 16 16" fill="currentColor">
          <path d="M6.2 1.8a6.2 6.2 0 108 8 5 5 0 01-8-8z" />
        </svg>
      {/if}
    </button>
  </div>
</div>

<style>
  #statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 22px;
    padding: 0 10px;
    background: var(--bg-chrome);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }

  .status-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.idle   { background: var(--text-faint); }
  .status-dot.active { background: var(--accent); }
  .status-dot.error  { background: var(--error); }

  #status {
    font-size: 11px;
    color: var(--text-dim);
  }

  .status-hint {
    font-size: 11px;
    color: var(--text-faint);
  }

  .theme-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 16px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--radius-xs);
    color: var(--text-dim);
    cursor: pointer;
  }

  .theme-toggle svg {
    width: 13px;
    height: 13px;
  }

  .theme-toggle:hover {
    background: var(--bg-hover);
    color: var(--accent);
  }

  .avatar-toggle.on {
    color: var(--accent);
  }
</style>
