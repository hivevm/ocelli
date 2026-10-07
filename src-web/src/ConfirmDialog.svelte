<script lang="ts">
  // The question before a write that discards work (ADR-0034): it names what is lost, and its
  // default button, the one Enter presses, keeps the work. Escape keeps it too.

  interface Props {
    /** What the write discards, in the core's words. */
    lost: string;
    /** The label of the button that goes ahead, such as "Delete tag". */
    action: string;
    onAnswer: (confirmed: boolean) => void;
  }
  let { lost, action, onAnswer }: Props = $props();

  let dialog = $state<HTMLDialogElement | null>(null);

  $effect(() => {
    dialog?.showModal();
  });
</script>

<dialog bind:this={dialog} class="confirm" oncancel={(e) => { e.preventDefault(); onAnswer(false); }}>
  <p class="question">This discards {lost}, and nothing brings it back.</p>
  <div class="actions">
    <button class="btn danger" onclick={() => onAnswer(true)}>{action}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button class="btn keep" autofocus onclick={() => onAnswer(false)}>Keep</button>
  </div>
</dialog>

<style>
  .confirm {
    max-width: 420px;
    padding: 16px;
    color: var(--text);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px var(--shadow);
  }

  .confirm::backdrop {
    background: var(--shadow);
  }

  .question {
    margin: 0 0 14px;
    color: var(--text-bright);
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

  .btn.keep {
    color: var(--accent);
    background: var(--accent-bg);
    border-color: var(--accent-border);
  }

  .btn.danger {
    color: var(--red);
  }

  .btn.danger:hover {
    background: var(--red-bg);
  }
</style>
