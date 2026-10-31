<script lang="ts">
  // The operation stopped in the repository, whoever started it, and its conflicted files
  // (ADR-0031): each resolved by ours, theirs, or as the developer edited it, then the operation
  // continued, or aborted after a confirmation.
  import { operationName, type Take, type Write } from './writes.ts';
  import type { InProgress } from './types.ts';

  interface Props {
    stopped: InProgress;
    /** Performs a write, asking first where it discards work; resolves with why it was refused. */
    onWrite: (write: Write) => Promise<string | null>;
  }
  let { stopped, onWrite }: Props = $props();

  let busy = $state(false);
  let refused = $state<string | null>(null);

  async function write(request: Write): Promise<void> {
    if (busy) return;
    busy = true;
    refused = await onWrite(request);
    busy = false;
  }

  function resolve(index: number, take: Take): void {
    const conflict = stopped.conflicts[index];
    void write({ kind: 'resolve', path: conflict.path, conflict, take });
  }
</script>

<div class="in-progress" role="region" aria-label="Operation in progress">
  <div class="headline">
    <span class="what">⚠ A {operationName(stopped.operation)} is in progress</span>
    <span class="count">
      {stopped.conflicts.length === 0 ? 'nothing conflicts any more' :
        `${stopped.conflicts.length} conflicted`}
    </span>
    <span class="actions">
      <button class="btn primary" disabled={busy || stopped.conflicts.length > 0}
              onclick={() => write({ kind: 'continue', operation: stopped.operation, head: stopped.head })}>
        Continue
      </button>
      <button class="btn" disabled={busy}
              onclick={() => write({ kind: 'abort', operation: stopped.operation, head: stopped.head })}>
        Abort…
      </button>
    </span>
  </div>
  {#each stopped.conflicts as conflict, i (conflict.path)}
    <div class="file">
      <span class="path" title={conflict.path}>{conflict.path}</span>
      <button class="btn" disabled={busy} onclick={() => resolve(i, 'ours')}>Ours</button>
      <button class="btn" disabled={busy} onclick={() => resolve(i, 'theirs')}>Theirs</button>
      <button class="btn" disabled={busy} title="Take the file as you edited it"
              onclick={() => resolve(i, 'asEdited')}>Resolved</button>
    </div>
  {/each}
  {#if refused}<div class="refused">{refused}</div>{/if}
</div>

<style>
  .in-progress {
    padding: 6px 12px;
    background: var(--amber-bg);
    border-bottom: 1px solid var(--amber-border);
    color: var(--text);
    flex-shrink: 0;
  }

  .headline,
  .file {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .what {
    color: var(--amber);
    font-weight: 600;
  }

  .count {
    color: var(--text-muted);
  }

  .actions {
    margin-left: auto;
    display: flex;
    gap: 6px;
  }

  .file {
    padding: 2px 0 2px 16px;
  }

  .path {
    font-family: var(--font-mono);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .refused {
    color: var(--error);
    font-size: 11px;
    padding-top: 4px;
  }

  .btn {
    font: inherit;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .btn.primary {
    color: var(--accent);
    background: var(--accent-bg);
    border-color: var(--accent-border);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
