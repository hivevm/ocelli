<script lang="ts">
  import { toSplitRows } from './diffSplit.ts';
  import type { DiffLine } from './types.ts';

  interface Props {
    lines: DiffLine[];
    filePath: string;
    onClose: () => void;
  }

  let { lines, filePath, onClose }: Props = $props();

  // Unified vs. side-by-side, remembered across files and sessions.
  const storedView = typeof localStorage !== 'undefined' ? localStorage.getItem('diffView') : null;
  let view = $state<'unified' | 'split'>(storedView === 'split' ? 'split' : 'unified');

  $effect(() => {
    try { localStorage.setItem('diffView', view); } catch { /* ignore */ }
  });

  const splitRows = $derived(view === 'split' ? toSplitRows(lines) : []);

  function fileName(path: string): string {
    return path.split('/').pop() ?? path;
  }
</script>

<div class="diff-panel">
  <div class="diff-header">
    <span class="diff-filepath">
      <span class="diff-dir">{filePath.includes('/') ? filePath.slice(0, filePath.lastIndexOf('/') + 1) : ''}</span>
      <span class="diff-filename">{fileName(filePath)}</span>
    </span>

    <div class="view-toggle">
      <button
        class="view-toggle-btn {view === 'unified' ? 'active' : ''}"
        title="Unified view"
        aria-label="Unified view"
        onclick={() => (view = 'unified')}
      >
        <svg viewBox="0 0 16 16" fill="currentColor">
          <rect x="2" y="3"    width="12" height="1.6" rx="0.8" />
          <rect x="2" y="7.2"  width="12" height="1.6" rx="0.8" />
          <rect x="2" y="11.4" width="12" height="1.6" rx="0.8" />
        </svg>
      </button>
      <button
        class="view-toggle-btn {view === 'split' ? 'active' : ''}"
        title="Side-by-side view"
        aria-label="Side-by-side view"
        onclick={() => (view = 'split')}
      >
        <svg viewBox="0 0 16 16" fill="currentColor">
          <rect x="2" y="3"    width="5" height="1.6" rx="0.8" />
          <rect x="9" y="3"    width="5" height="1.6" rx="0.8" />
          <rect x="2" y="7.2"  width="5" height="1.6" rx="0.8" />
          <rect x="9" y="7.2"  width="5" height="1.6" rx="0.8" />
          <rect x="2" y="11.4" width="5" height="1.6" rx="0.8" />
          <rect x="9" y="11.4" width="5" height="1.6" rx="0.8" />
        </svg>
      </button>
    </div>

    <button class="diff-close" onclick={onClose} title="Close diff">✕</button>
  </div>

  <div class="diff-body">
    {#if lines.length === 0}
      <div class="diff-empty">No changes</div>
    {:else if view === 'unified'}
      <table class="diff-table">
        <tbody>
          {#each lines as line}
            {#if line.kind === 'hunk' || line.kind === 'meta'}
              <tr class="hunk-row">
                <td class="ln ln-old"></td>
                <td class="ln ln-new"></td>
                <td class="hunk-header">{line.content}</td>
              </tr>
            {:else}
              <tr class="line-row {line.kind}">
                <td class="ln ln-old">{line.oldLineno ?? ''}</td>
                <td class="ln ln-new">{line.newLineno ?? ''}</td>
                <td class="line-content">
                  <span class="line-sign">
                    {#if line.kind === 'add'}+{:else if line.kind === 'delete'}-{:else} {/if}
                  </span><span class="line-text">{line.content}</span>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {:else}
      <table class="diff-table split">
        <colgroup>
          <col class="col-ln" />
          <col />
          <col class="col-ln" />
          <col />
        </colgroup>
        <tbody>
          {#each splitRows as row}
            {#if row.kind === 'banner'}
              <tr class="hunk-row">
                <td class="ln"></td>
                <td class="hunk-header" colspan="3">{row.content}</td>
              </tr>
            {:else}
              <tr class="split-line">
                <td class="ln cell-{row.left.kind}">{row.left.lineno ?? ''}</td>
                <td class="split-content cell-{row.left.kind}">{row.left.content}</td>
                <td class="ln split-divider cell-{row.right.kind}">{row.right.lineno ?? ''}</td>
                <td class="split-content cell-{row.right.kind}">{row.right.content}</td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

<style>
  .diff-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-panel);
    overflow: hidden;
  }

  .diff-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px;
    height: 36px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    gap: 8px;
  }

  .diff-filepath {
    font-family: var(--font-mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .diff-dir      { color: var(--text-dimmer); }
  .diff-filename { color: var(--text-2); font-weight: 600; }

  .view-toggle {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }

  .view-toggle-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 18px;
    padding: 0;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--text-dimmer);
    cursor: pointer;
  }

  .view-toggle-btn svg {
    width: 13px;
    height: 13px;
  }

  .view-toggle-btn:hover {
    background: var(--bg-hover);
    color: var(--text-muted);
  }

  .view-toggle-btn.active {
    background: var(--accent-bg);
    border-color: var(--accent-border);
    color: var(--accent);
  }

  .diff-close {
    background: none;
    border: none;
    color: var(--text-dimmer);
    cursor: pointer;
    font-size: 13px;
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    flex-shrink: 0;
    line-height: 1;
  }
  .diff-close:hover { background: var(--bg-elev); color: var(--text-2); }

  .diff-body {
    flex: 1;
    overflow: auto;
  }

  .diff-empty {
    padding: 20px;
    color: var(--text-dimmer);
    font-size: 12px;
    text-align: center;
  }

  .diff-table {
    border-collapse: collapse;
    width: 100%;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 20px;
    white-space: pre;
    table-layout: fixed;
  }

  .ln {
    width: 44px;
    min-width: 44px;
    padding: 0 8px;
    text-align: right;
    color: var(--text-faint);
    user-select: none;
    border-right: 1px solid var(--border);
    background: var(--diff-gutter-bg);
    vertical-align: top;
  }

  .line-content {
    padding: 0 0 0 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    vertical-align: top;
  }

  .line-sign {
    display: inline-block;
    width: 14px;
    user-select: none;
  }

  .line-text {
    color: var(--text-2);
  }

  /* Add rows */
  .line-row.add .ln          { background: var(--diff-add-bg); color: var(--diff-add-gutter); border-right-color: var(--diff-add-border); }
  .line-row.add .line-content { background: var(--diff-add-bg); }
  .line-row.add .line-sign   { color: var(--accent); }
  .line-row.add .line-text   { color: var(--diff-add-text); }

  /* Delete rows */
  .line-row.delete .ln          { background: var(--diff-del-bg); color: var(--diff-del-gutter); border-right-color: var(--diff-del-border); }
  .line-row.delete .line-content { background: var(--diff-del-bg); }
  .line-row.delete .line-sign   { color: var(--red); }
  .line-row.delete .line-text   { color: var(--diff-del-text); }

  /* Context rows */
  .line-row.context .line-content { background: var(--bg-panel); }
  .line-row.context .line-sign    { color: var(--text-faint); }

  /* Hunk header */
  .hunk-row .hunk-header {
    padding: 0 8px;
    background: var(--diff-hunk-bg);
    color: var(--diff-hunk-text);
    font-style: italic;
    white-space: pre;
  }
  .hunk-row .ln { background: var(--diff-hunk-gutter-bg); }

  /* ── Side-by-side view ─────────────────────────────────────────────────── */
  .diff-table.split .col-ln {
    width: 44px;
  }

  .split-content {
    padding: 0 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    vertical-align: top;
    color: var(--text-2);
  }

  .split-divider {
    border-left: 1px solid var(--border);
  }

  td.cell-add {
    background: var(--diff-add-bg);
    color: var(--diff-add-text);
  }
  .ln.cell-add {
    color: var(--diff-add-gutter);
    border-right-color: var(--diff-add-border);
  }

  td.cell-delete {
    background: var(--diff-del-bg);
    color: var(--diff-del-text);
  }
  .ln.cell-delete {
    color: var(--diff-del-gutter);
    border-right-color: var(--diff-del-border);
  }

  td.cell-empty {
    background: var(--diff-gutter-bg);
  }
</style>
