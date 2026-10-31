<script lang="ts">
  // The remote server's counterpart of the desktop's folder dialog (ADR-0035, ADR-0037): the
  // folders below the server root, one folder at a time, those holding a `.git` marked; opening
  // one hands its path to the same load the sidebar starts. The server decides what opens.
  import Dialog from './Dialog.svelte';
  import { parentFolder } from './folders.ts';
  import { invoke } from './host.ts';
  import type { Listing } from './types.ts';

  interface Props {
    /** Opens the repository at `path`, relative to the server root. */
    onOpen: (path: string) => void;
    onClose: () => void;
  }
  let { onOpen, onClose }: Props = $props();

  let listing = $state<Listing | null>(null);
  let error = $state<string | null>(null);
  /** The path of the listing asked for last: an answer to an earlier one is dropped. */
  let asked = '.';

  async function show(path: string): Promise<void> {
    asked = path;
    error = null;
    try {
      const answer = await invoke<Listing>('list_folders', { path });
      if (asked === path) listing = answer;
    } catch (e: unknown) {
      if (asked === path) error = String(e);
    }
  }

  $effect(() => {
    void show('.');
  });

  function open(path: string): void {
    onOpen(path);
    onClose();
  }

  const up = $derived(listing ? parentFolder(listing.path) : null);
</script>

<Dialog onCancel={onClose} width="420px">
  <div class="title">Open a repository below the server's folder</div>
  <div class="where">{listing?.path ?? '.'}</div>
  <div class="list">
    {#if up !== null}
      <button class="row" onclick={() => show(up)}>..</button>
    {/if}
    {#each listing?.folders ?? [] as folder (folder.path)}
      <div class="entry">
        <button class="row" onclick={() => show(folder.path)} title="Show the folders in {folder.name}">
          {folder.name}
        </button>
        {#if folder.repository}
          <button class="btn primary" onclick={() => open(folder.path)}>Open</button>
        {/if}
      </div>
    {/each}
    {#if listing && listing.folders.length === 0}<div class="note">No folders here</div>{/if}
    {#if listing?.more}<div class="note">More entries than the server lists</div>{/if}
  </div>
  {#if error}<div class="error">{error}</div>{/if}
  <div class="actions">
    <button type="button" class="btn" onclick={onClose}>Cancel</button>
    {#if listing?.repository}
      <button type="button" class="btn primary" onclick={() => listing && open(listing.path)}>
        Open this folder
      </button>
    {/if}
  </div>
</Dialog>

<style>
  .title {
    color: var(--text-bright);
    margin-bottom: 6px;
  }

  .where {
    color: var(--text-muted);
    margin-bottom: 6px;
    overflow-wrap: anywhere;
  }

  .list {
    display: flex;
    flex-direction: column;
    max-height: 50vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }

  .entry {
    display: flex;
    align-items: center;
  }

  .row {
    flex: 1;
    text-align: left;
    font: inherit;
    color: var(--text);
    background: none;
    border: none;
    padding: 4px 8px;
    cursor: pointer;
  }

  .row:hover {
    background: var(--bg-elev);
  }

  .note {
    color: var(--text-muted);
    padding: 4px 8px;
  }

  .error {
    color: var(--error);
    margin-top: 6px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }

  .btn {
    font: inherit;
    color: var(--text);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 3px 10px;
    cursor: pointer;
  }

  .btn.primary {
    color: var(--text-bright);
    border-color: var(--accent-border-strong);
  }
</style>
