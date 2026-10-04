<script lang="ts">
  import FileTree from './FileTree.svelte';
  import { buildFileTree } from './fileTree.ts';
  import type { NodeJson, FileChange } from './types.ts';

  interface Props {
    node: NodeJson | null;
    nodeBySha: Map<string, NodeJson>;
    showAvatars: boolean;
    changedFiles: FileChange[];
    diffFile: string | null;
    onSelectFile: (file: FileChange) => void;
    onJumpToSha: (sha: string) => void;
  }
  let { node, nodeBySha, showAvatars, changedFiles, diffFile, onSelectFile, onJumpToSha }: Props =
    $props();

  let fileViewMode = $state<'flat' | 'tree'>('tree');
  const fileTree = $derived(buildFileTree(changedFiles));

  // Copy-to-clipboard feedback; reset when another commit is selected.
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    void node;
    copied = false;
  });

  async function copySha(): Promise<void> {
    if (!node || node.kind === 'working') return;
    try {
      await navigator.clipboard.writeText(node.sha);
      copied = true;
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copied = false), 1500);
    } catch {
      /* clipboard unavailable — nothing sensible to do */
    }
  }

  /** Link label for a parent/child SHA. Synthetic nodes have no useful short
      sha — the working-directory node's is empty — so they get a name instead. */
  function relativeLabel(sha: string): string {
    const n = nodeBySha.get(sha);
    if (!n) return sha.slice(0, 7);
    if (n.kind === 'working') return 'Uncommitted';
    return n.shortSha;
  }
</script>

<div id="sidebar">
  <div class="sidebar-section-header">COMMIT</div>

  {#if node}
    <div class="detail-block">
      {#if node.kind === 'working'}
        <div class="detail-sha">Uncommitted changes</div>
      {:else}
        <button class="detail-sha copyable" title="Click to copy the full SHA" onclick={copySha}>
          {node.sha}
          {#if copied}<span class="copied-badge">copied</span>{/if}
        </button>
      {/if}

      {#if node.refs.length > 0}
        <div class="detail-refs">
          {#each node.refs as ref}
            <span class="ref-chip {ref.startsWith('🏷') ? 'tag' : ref.includes('/') ? 'remote' : 'local'}">
              {ref}
            </span>
          {/each}
        </div>
      {/if}

      <div class="detail-message">{node.message}</div>
    </div>

    <div class="sidebar-divider"></div>
    <div class="sidebar-section-header">AUTHOR</div>
    <div class="detail-block detail-author-row">
      <div class="detail-author-avatar">
        {node.author.charAt(0).toUpperCase()}
        {#if showAvatars && node.authorAvatar}
          <img
            class="avatar-img"
            src={node.authorAvatar}
            alt=""
            onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = 'none')}
          />
        {/if}
      </div>
      <div class="detail-author-info">
        <div class="detail-author-name">{node.author}</div>
        <div class="detail-date">{node.authorDate}</div>
      </div>
    </div>

    {#if node.kind !== 'working' && (node.committer !== node.author || node.date !== node.authorDate)}
      <div class="sidebar-divider"></div>
      <div class="sidebar-section-header">COMMITTER</div>
      <div class="detail-block detail-author-row">
        <div class="detail-author-avatar">
          {node.committer.charAt(0).toUpperCase()}
          {#if showAvatars && node.committerAvatar}
            <img
              class="avatar-img"
              src={node.committerAvatar}
              alt=""
              onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = 'none')}
            />
          {/if}
        </div>
        <div class="detail-author-info">
          <div class="detail-author-name">{node.committer}</div>
          <div class="detail-date">{node.date}</div>
        </div>
      </div>
    {/if}

    {#if node.parents.length > 0}
      <div class="sidebar-divider"></div>
      <div class="sidebar-section-header">
        <span>PARENTS</span>
        <span class="file-count">{node.parents.length}</span>
      </div>
      <div class="detail-block detail-relatives">
        {#each node.parents as parentSha (parentSha)}
          {#if nodeBySha.has(parentSha)}
            <button class="sha-link" onclick={() => onJumpToSha(parentSha)}>
              {relativeLabel(parentSha)}
            </button>
          {:else}
            <span class="sha-link unloaded" title="Not loaded — outside the commit limit">
              {parentSha.slice(0, 7)}
            </span>
          {/if}
        {/each}
      </div>
    {/if}

    {#if node.children.length > 0}
      <div class="sidebar-divider"></div>
      <div class="sidebar-section-header">
        <span>CHILDREN</span>
        <span class="file-count">{node.children.length}</span>
      </div>
      <div class="detail-block detail-relatives">
        {#each node.children as childSha (childSha)}
          <button class="sha-link" onclick={() => onJumpToSha(childSha)}>
            {relativeLabel(childSha)}
          </button>
        {/each}
      </div>
    {/if}

    <div class="sidebar-divider"></div>
    <div class="sidebar-section-header">
      <span>FILES CHANGED</span>
      {#if changedFiles.length > 0}
        <span class="file-count">{changedFiles.length}</span>
      {/if}
      <div class="view-toggle">
        <button
          class="view-toggle-btn {fileViewMode === 'flat' ? 'active' : ''}"
          title="Flat view"
          aria-label="Flat view"
          onclick={() => (fileViewMode = 'flat')}
        >
          <svg viewBox="0 0 16 16" fill="currentColor">
            <rect x="2" y="3"  width="12" height="1.6" rx="0.8" />
            <rect x="2" y="7.2" width="12" height="1.6" rx="0.8" />
            <rect x="2" y="11.4" width="12" height="1.6" rx="0.8" />
          </svg>
        </button>
        <button
          class="view-toggle-btn {fileViewMode === 'tree' ? 'active' : ''}"
          title="Tree view"
          aria-label="Tree view"
          onclick={() => (fileViewMode = 'tree')}
        >
          <svg viewBox="0 0 16 16" fill="currentColor">
            <rect x="2" y="3"  width="10" height="1.6" rx="0.8" />
            <rect x="5" y="7.2" width="9" height="1.6" rx="0.8" />
            <rect x="5" y="11.4" width="9" height="1.6" rx="0.8" />
          </svg>
        </button>
      </div>
    </div>

    {#if changedFiles.length === 0}
      <div class="files-loading">
        <div class="spinner"></div>
      </div>
    {:else if fileViewMode === 'tree'}
      <div class="file-list">
        <FileTree nodes={fileTree} {diffFile} {onSelectFile} />
      </div>
    {:else}
      <div class="file-list">
        {#each changedFiles as file}
          <div
            class="file-row {diffFile === file.path ? 'selected' : ''}"
            onclick={() => onSelectFile(file)}
            role="button"
            tabindex="0"
            onkeydown={(e) => e.key === 'Enter' && onSelectFile(file)}
          >
            <span class="file-status {file.status}">{file.status[0]}</span>
            <span class="file-path" title={file.path}>
              {#if file.oldPath}
                <span class="file-old-path">{file.oldPath}</span>
                <span class="rename-arrow">→</span>
              {/if}
              {file.path}
            </span>
            {#if file.additions > 0 || file.deletions > 0}
              <span class="file-stats">
                {#if file.additions > 0}<span class="stat-add">+{file.additions}</span>{/if}
                {#if file.deletions > 0}<span class="stat-del">-{file.deletions}</span>{/if}
              </span>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <div class="sidebar-empty">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="3"/>
        <path d="M3 12h3m12 0h3M12 3v3m0 12v3"/>
      </svg>
      <p>Select a commit to view details</p>
    </div>
  {/if}
</div>

<style>
  #sidebar {
    width: 280px;
    flex-shrink: 0;
    background: var(--bg-chrome);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .sidebar-section-header {
    padding: 10px 14px 6px;
    font-size: 10px;
    font-weight: 700;
    color: var(--text-dimmer);
    letter-spacing: 0.08em;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sidebar-divider {
    height: 1px;
    background: var(--border);
    margin: 8px 0;
    flex-shrink: 0;
  }

  .detail-block {
    padding: 4px 14px 10px;
    flex-shrink: 0;
  }

  .detail-sha {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--blue);
    word-break: break-all;
    margin-bottom: 8px;
    line-height: 1.5;
  }

  button.detail-sha {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
  }

  button.detail-sha:hover {
    text-decoration: underline;
  }

  .copied-badge {
    font-family: var(--font-ui);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent);
    background: var(--accent-bg);
    border: 1px solid var(--accent-border);
    border-radius: var(--radius-xs);
    padding: 1px 5px;
    margin-left: 6px;
    white-space: nowrap;
  }

  .detail-refs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-bottom: 10px;
  }

  .ref-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    font-size: 10px;
    font-weight: 500;
    line-height: 16px;
    white-space: nowrap;
  }

  .ref-chip.local  { background: var(--accent-bg); color: var(--accent); border: 1px solid var(--accent-border); }
  .ref-chip.remote { background: var(--blue-bg); color: var(--blue); border: 1px solid var(--blue-border); }
  .ref-chip.tag    { background: var(--amber-bg); color: var(--amber); border: 1px solid var(--amber-border); }

  .detail-relatives {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .sha-link {
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 16px;
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    background: var(--blue-bg);
    color: var(--blue);
    border: 1px solid var(--blue-border);
    white-space: nowrap;
  }

  button.sha-link {
    cursor: pointer;
  }

  button.sha-link:hover {
    border-color: var(--border-hover);
    filter: brightness(1.2);
  }

  .sha-link.unloaded {
    background: transparent;
    color: var(--text-dim);
    border-color: var(--border);
  }

  .detail-message {
    font-size: 13px;
    color: var(--text);
    line-height: 1.55;
    word-break: break-word;
  }

  .detail-author-avatar {
    position: relative;
    overflow: hidden;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--accent-bg-strong), var(--accent-border-strong));
    color: var(--accent);
    font-size: 15px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 8px;
    flex-shrink: 0;
  }

  .detail-author-avatar .avatar-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .detail-author-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .detail-author-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
  }

  .detail-date {
    font-size: 11px;
    color: var(--text-dim);
    font-family: var(--font-mono);
  }

  .detail-author-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .file-count {
    background: var(--bg-elev);
    color: var(--text-dim);
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: var(--radius-lg);
    letter-spacing: 0;
    line-height: 14px;
  }

  .view-toggle {
    display: flex;
    gap: 2px;
    margin-left: auto;
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

  .files-loading {
    display: flex;
    justify-content: center;
    padding: 16px;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .file-list {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .file-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 14px;
    font-size: 11px;
    cursor: pointer;
    min-width: 0;
  }

  .file-row:hover {
    background: var(--bg-hover);
  }

  .file-row.selected {
    background: var(--bg-sel);
    border-left: 2px solid var(--accent);
    padding-left: 12px;
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

  .sidebar-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-faint);
    padding: 40px 20px;
    text-align: center;
  }

  .sidebar-empty svg {
    width: 32px;
    height: 32px;
    color: var(--border);
  }

  .sidebar-empty p {
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dimmer);
  }
</style>
