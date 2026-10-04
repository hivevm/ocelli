<script lang="ts">
  import type { TreeNode } from './fileTree.ts';
  import type { FileChange } from './types.ts';
  import Self from './FileTree.svelte';

  interface Props {
    nodes: TreeNode[];
    depth?: number;
    diffFile: string | null;
    onSelectFile: (file: FileChange) => void;
  }
  let { nodes, depth = 0, diffFile, onSelectFile }: Props = $props();

  // Collapsed directory paths (expanded by default).
  let collapsed = $state<Record<string, boolean>>({});
  function toggle(path: string): void {
    collapsed[path] = !collapsed[path];
  }

  const indent = (d: number) => 14 + d * 12;
</script>

{#each nodes as node (node.path)}
  {#if node.type === 'dir'}
    <div
      class="dir-row"
      style="padding-left: {indent(depth)}px"
      onclick={() => toggle(node.path)}
      role="button"
      tabindex="0"
      onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && toggle(node.path)}
    >
      <svg class="chevron {collapsed[node.path] ? '' : 'open'}" viewBox="0 0 16 16" fill="currentColor">
        <path d="M6 4l4 4-4 4z" />
      </svg>
      <svg class="folder-icon" viewBox="0 0 16 16" fill="currentColor">
        <path d="M1.5 3A1.5 1.5 0 013 1.5h3.379a1.5 1.5 0 011.06.44L8.5 3h4A1.5 1.5 0 0114 4.5v7A1.5 1.5 0 0112.5 13h-9A1.5 1.5 0 012 11.5v-8z" />
      </svg>
      <span class="dir-name" title={node.path}>{node.name}</span>
      <span class="dir-count">{node.fileCount}</span>
    </div>
    {#if !collapsed[node.path]}
      <Self nodes={node.children} depth={depth + 1} {diffFile} {onSelectFile} />
    {/if}
  {:else}
    <div
      class="file-row {diffFile === node.path ? 'selected' : ''}"
      style="padding-left: {indent(depth)}px"
      onclick={() => onSelectFile(node.file)}
      role="button"
      tabindex="0"
      onkeydown={(e) => e.key === 'Enter' && onSelectFile(node.file)}
    >
      <span class="file-spacer"></span>
      <span class="file-status {node.file.status}">{node.file.status[0]}</span>
      <span class="file-path" title={node.file.path}>
        {#if node.file.oldPath}
          <span class="file-old-path">{node.file.oldPath.split('/').pop()}</span>
          <span class="rename-arrow">→</span>
        {/if}
        {node.name}
      </span>
      {#if node.file.additions > 0 || node.file.deletions > 0}
        <span class="file-stats">
          {#if node.file.additions > 0}<span class="stat-add">+{node.file.additions}</span>{/if}
          {#if node.file.deletions > 0}<span class="stat-del">-{node.file.deletions}</span>{/if}
        </span>
      {/if}
    </div>
  {/if}
{/each}

<style>
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 14px;
    min-height: 22px;
    cursor: pointer;
    font-size: 11px;
    color: var(--text-2);
    min-width: 0;
  }

  /* Empty slot in file rows that matches the chevron, so a file's
     status icon lines up with the folder icon at the same depth. */
  .file-spacer {
    width: 12px;
    flex-shrink: 0;
  }

  .dir-row:hover {
    background: var(--bg-hover);
  }

  .chevron {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    color: var(--text-dim);
    transition: transform 0.1s;
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .folder-icon {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    color: var(--text-muted);
  }

  .dir-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dir-count {
    flex-shrink: 0;
    background: var(--bg-elev);
    color: var(--text-dim);
    font-size: 9px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: var(--radius-lg);
    line-height: 14px;
    font-family: var(--font-mono);
  }

  .file-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 14px;
    min-height: 22px;
    font-size: 11px;
    cursor: pointer;
    min-width: 0;
  }

  .file-row:hover {
    background: var(--bg-hover);
  }

  .file-row.selected {
    background: var(--bg-sel);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .file-status {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-xs);
    font-size: 9px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    text-transform: uppercase;
  }

  .file-status.added    { background: var(--accent-bg); color: var(--accent); }
  .file-status.modified { background: var(--blue-bg); color: var(--blue); }
  .file-status.deleted  { background: var(--red-bg); color: var(--red); }
  .file-status.renamed  { background: var(--amber-bg); color: var(--amber); }
  .file-status.copied   { background: var(--teal-bg); color: var(--teal); }
  .file-status.unknown  { background: var(--bg-elev); color: var(--text-dim); }

  .file-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 10.5px;
  }

  .file-old-path {
    color: var(--text-dim);
    text-decoration: line-through;
  }

  .rename-arrow {
    color: var(--text-dimmer);
    margin: 0 2px;
  }

  .file-stats {
    display: flex;
    gap: 3px;
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 10px;
  }

  .stat-add { color: var(--accent); }
  .stat-del { color: var(--red); }
</style>
