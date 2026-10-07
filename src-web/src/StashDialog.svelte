<script lang="ts">
  // Describes a new stash (G-7): a message, and whether the untracked files go with the changes;
  // ignored files never do. A refusal, such as nothing to stash, is said here.
  import { stashRequest, type Write } from './writes.ts';

  interface Props {
    /** Asks for the stash; resolves with why it was refused, or null once written. */
    onStash: (write: Write) => Promise<string | null>;
    onClose: () => void;
  }
  let { onStash, onClose }: Props = $props();

  let dialog = $state<HTMLDialogElement | null>(null);
  let message = $state('');
  let untracked = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);

  $effect(() => {
    dialog?.showModal();
  });

  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = await onStash(stashRequest(message, untracked));
    busy = false;
    if (error === null) onClose();
  }
</script>

<dialog bind:this={dialog} class="stash" oncancel={(e) => { e.preventDefault(); onClose(); }}>
  <form onsubmit={submit}>
    <div class="title">Stash the uncommitted changes</div>
    <!-- svelte-ignore a11y_autofocus -->
    <input class="field" placeholder="Message (optional)" aria-label="Stash message" autofocus
           bind:value={message} />
    <label class="switch">
      <input type="checkbox" bind:checked={untracked} />
      Include untracked files
    </label>
    {#if error}<div class="error">{error}</div>{/if}
    <div class="actions">
      <button type="button" class="btn" onclick={onClose}>Cancel</button>
      <button type="submit" class="btn primary" disabled={busy}>Stash</button>
    </div>
  </form>
</dialog>

<style>
  .stash {
    width: 380px;
    padding: 16px;
    color: var(--text);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px var(--shadow);
  }

  .stash::backdrop {
    background: var(--shadow);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .title {
    color: var(--text-bright);
  }

  .field {
    font: inherit;
    color: var(--text);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 4px 6px;
  }

  .field:focus {
    outline: none;
    border-color: var(--accent-border-strong);
  }

  .switch {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .error {
    color: var(--error);
    font-size: 11px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .btn {
    font: inherit;
    padding: 4px 12px;
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
