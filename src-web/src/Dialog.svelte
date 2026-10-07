<script lang="ts">
  // The shell every dialog of the web interface opens in: modal, so the page under it takes no
  // input; Escape is its cancel; its backdrop and shadow come from the theme's tokens.
  import type { Snippet } from 'svelte';

  interface Props {
    /** What Escape does: the dialog's own cancel. */
    onCancel: () => void;
    /** A fixed width; without one the dialog fits its content, up to 420px. */
    width?: string;
    children: Snippet;
  }
  let { onCancel, width, children }: Props = $props();

  let dialog = $state<HTMLDialogElement | null>(null);

  $effect(() => {
    dialog?.showModal();
  });
</script>

<dialog bind:this={dialog} style:width oncancel={(e) => { e.preventDefault(); onCancel(); }}>
  {@render children()}
</dialog>

<style>
  dialog {
    max-width: min(420px, calc(100vw - 32px));
    padding: 16px;
    color: var(--text);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px var(--shadow);
  }

  dialog::backdrop {
    background: var(--shadow);
  }
</style>
