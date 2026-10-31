<script lang="ts">
  import FileTree from './FileTree.svelte';
  import StageToggle from './StageToggle.svelte';
  import { buildFileTree } from './fileTree.ts';
  import {
    OneWrite, branchRequest, commitRequest, stageToggle, tagRequest, type ResetMode, type Write,
    type WriteOf,
  } from './writes.ts';
  import type { Chip } from './refChips.ts';
  import type { NodeJson, FileChange, Relative } from './types.ts';

  interface Props {
    node: NodeJson | null;
    /** The parents and children of `node`, by SHA, as the host describes them. */
    relatives: Map<string, Relative>;
    showAvatars: boolean;
    changedFiles: FileChange[];
    diffFile: string | null;
    onSelectFile: (file: FileChange) => void;
    onJumpToRow: (row: number) => void;
    /** The ref chips of `node`, each with what it offers. */
    chips: Chip[];
    /** The operations of G-13 the commit offers on the checked-out branch, as the core decides. */
    operations: Write[];
    /** Performs a write on `node` (G-16), asking first where it discards work (ADR-0034);
        resolves with why it was refused, or null. */
    onWrite: (write: Write) => Promise<string | null>;
  }
  let {
    node, relatives, showAvatars, changedFiles, diffFile, onSelectFile, onJumpToRow, chips, operations,
    onWrite,
  }: Props = $props();

  // The reset's mode, mixed as git's default.
  let resetMode = $state<ResetMode>('mixed');

  /** Runs an operation of G-13 the core offered, a reset in the mode chosen (ADR-0033). */
  function run(action: Write): void {
    void write(action.kind === 'reset' ? { ...action, mode: resetMode } : action);
  }

  /** The offered operation of `kind`, where the commit offers it. */
  function offered<K extends Write['kind']>(kind: K): WriteOf<K> | undefined {
    return operations.find((a): a is WriteOf<K> => a.kind === kind);
  }

  /** The place of `node` in the stash, when it is a stash node. */
  const stash = $derived(node?.stash ?? null);

  // The one write the form, the buttons, and the chips run at a time, and why the last was
  // refused, shown with the commit it was asked for.
  const one = new OneWrite();
  let writing = $state(false);
  let writeError = $state<string | null>(null);

  /** Runs `request`; resolves with whether it was written or kept, for this commit. */
  async function write(request: Write): Promise<boolean> {
    if (!node) return false;
    const sha = node.sha;
    writing = true;
    const answer = await one.run(sha, () => onWrite(request), () => node?.sha);
    writing = one.running;
    if (!answer) return false;
    writeError = answer.error;
    return answer.error === null;
  }

  function onStash(drop: boolean | 'only'): void {
    if (!node || stash === null) return;
    const commit = node.sha;
    void write(drop === 'only'
      ? { kind: 'dropStash', index: stash, commit }
      : { kind: 'applyStash', index: stash, commit, drop });
  }

  /** Stages or unstages a file of the uncommitted changes. */
  function onStage(file: FileChange): void {
    const toggle = stageToggle(file);
    if (toggle) void write(toggle);
  }

  // The commit form of the working-directory node, on its parent, the checked-out commit.
  let commitMessage = $state('');
  const commit = $derived(
    node?.kind === 'working' ? commitRequest(node.parents[0] ?? null, commitMessage) : null,
  );
  const anyStaged = $derived(
    changedFiles.some((f) => f.staging?.staged === 'yes' || f.staging?.staged === 'partly'),
  );

  async function submitCommit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (commit && (await write(commit))) commitMessage = '';
  }

  // The branch form: a name, and whether to switch to the branch.
  let branchName = $state('');
  let switchTo = $state(true);
  const branch = $derived(node ? branchRequest(node.sha, branchName, switchTo) : null);

  async function submitBranch(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (branch && (await write(branch))) form = null;
  }

  let fileViewMode = $state<'flat' | 'tree'>('tree');
  const fileTree = $derived(buildFileTree(changedFiles));

  // Copy-to-clipboard feedback; reset when another commit is selected.
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  // The tag form, closed and emptied when another commit is selected.
  let form = $state<'tag' | 'branch' | null>(null);
  let tagName = $state('');
  let tagMessage = $state('');
  const tag = $derived(node ? tagRequest(node.sha, tagName, tagMessage) : null);

  $effect(() => {
    void node;
    copied = false;
    form = null;
    tagName = '';
    tagMessage = '';
    branchName = '';
    switchTo = true;
    writing = one.running;
    writeError = null;
    // A reload after staging selects the working-directory node anew; its message stays.
    if (node?.kind !== 'working') commitMessage = '';
  });

  /** Opens a form, or closes it; a refusal shown before belongs to what came before. */
  function openForm(open: 'tag' | 'branch' | null): void {
    form = open;
    writeError = null;
  }

  async function submitTag(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (tag && (await write(tag))) form = null;
  }

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

  /** Link label for a parent/child SHA; the host names synthetic nodes, whose
      short sha is of no use. */
  function relativeLabel(sha: string): string {
    return relatives.get(sha)?.label ?? sha.slice(0, 7);
  }

  /** The row of a parent/child, null while it has not loaded. */
  function relativeRow(sha: string): number | null {
    return relatives.get(sha)?.row ?? null;
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

      {#if chips.length > 0}
        <div class="detail-refs">
          {#each chips as chip}
            <span class="ref-chip {chip.kind}">
              {chip.ref}
              {#each chip.offers as offer}
                <button class="chip-delete" disabled={writing} title={offer.label}
                        aria-label={offer.label}
                        onclick={() => void write(offer.write)}>{offer.action === 'switch' ? '⇄' : '×'}</button>
              {/each}
            </span>
          {/each}
        </div>
      {/if}

      <div class="detail-message">{node.message}</div>

      {#if node.kind === 'working'}
        <form class="tag-form" onsubmit={submitCommit}>
          <textarea class="tag-input" rows="3" aria-label="Commit message"
                    placeholder={anyStaged ? 'Commit message' : 'Stage files below to commit them'}
                    bind:value={commitMessage}></textarea>
          <div class="tag-actions">
            <button type="submit" class="tag-btn primary" disabled={!commit || !anyStaged || writing}>
              Commit
            </button>
          </div>
        </form>
      {/if}

      {#if stash !== null}
        <div class="tag-actions stash-actions">
          <button class="tag-btn" disabled={writing} onclick={() => onStash(false)}>Apply</button>
          <button class="tag-btn" disabled={writing} onclick={() => onStash(true)}>Pop…</button>
          <button class="tag-btn" disabled={writing} onclick={() => onStash('only')}>Drop…</button>
        </div>
      {/if}
      {#if writeError && form === null}<div class="tag-error">{writeError}</div>{/if}

      {#if node.kind === 'commit'}
        {#if form === 'tag'}
          <form class="tag-form" onsubmit={submitTag}>
            <!-- svelte-ignore a11y_autofocus -->
            <input class="tag-input" placeholder="Tag name" aria-label="Tag name" autofocus
                   bind:value={tagName} />
            <textarea class="tag-input" rows="2" aria-label="Tag message"
                      placeholder="Message — leave empty for a lightweight tag"
                      bind:value={tagMessage}></textarea>
            {#if writeError}<div class="tag-error">{writeError}</div>{/if}
            <div class="tag-actions">
              <button type="button" class="tag-btn" onclick={() => openForm(null)}>Cancel</button>
              <button type="submit" class="tag-btn primary" disabled={!tag || writing}>
                {tag?.message === null ? 'Create lightweight tag' : 'Create tag'}
              </button>
            </div>
          </form>
        {:else}
          {#if form === 'branch'}
            <form class="tag-form" onsubmit={submitBranch}>
              <!-- svelte-ignore a11y_autofocus -->
              <input class="tag-input" placeholder="Branch name" aria-label="Branch name" autofocus
                     bind:value={branchName} />
              <label class="switch-to">
                <input type="checkbox" bind:checked={switchTo} /> Switch to it
              </label>
              {#if writeError}<div class="tag-error">{writeError}</div>{/if}
              <div class="tag-actions">
                <button type="button" class="tag-btn" onclick={() => openForm(null)}>Cancel</button>
                <button type="submit" class="tag-btn primary" disabled={!branch || writing}>
                  Create branch
                </button>
              </div>
            </form>
          {:else}
            <div class="tag-actions commit-actions">
              <button class="tag-btn" onclick={() => openForm('tag')}>Tag…</button>
              <button class="tag-btn" onclick={() => openForm('branch')}>Branch…</button>
              <button class="tag-btn" disabled={writing} title="Check this commit out, HEAD detached"
                      onclick={() => node && void write({ kind: 'detach', commit: node.sha })}>
                Switch here
              </button>
            </div>
            {#if operations.length > 0}
              <div class="tag-actions commit-actions">
                {#each operations.filter(a => a.kind !== 'reset') as action (action.kind)}
                  {#if action.kind === 'merge'}
                    <button class="tag-btn" disabled={writing} onclick={() => run(action)}
                            title="Merge this commit into {action.branch}">Merge into {action.branch}</button>
                  {:else if action.kind === 'rebase'}
                    <button class="tag-btn" disabled={writing} onclick={() => run(action)}
                            title="Replay {action.branch} onto this commit">Rebase onto</button>
                  {:else if action.kind === 'cherryPick'}
                    <button class="tag-btn" disabled={writing} onclick={() => run(action)}>Cherry-pick</button>
                  {:else if action.kind === 'revert'}
                    <button class="tag-btn" disabled={writing} onclick={() => run(action)}>Revert</button>
                  {/if}
                {/each}
              </div>
              {@const reset = offered('reset')}
              {#if reset}
                <div class="tag-actions commit-actions">
                  <select class="tag-input" aria-label="Reset mode" bind:value={resetMode}>
                    <option value="soft">soft</option>
                    <option value="mixed">mixed</option>
                    <option value="hard">hard</option>
                  </select>
                  <button class="tag-btn" disabled={writing} onclick={() => run(reset)}
                          title="Move {reset.branch} to this commit">Reset {reset.branch} here</button>
                </div>
              {/if}
            {/if}
          {/if}
        {/if}
      {/if}
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
          {@const row = relativeRow(parentSha)}
          {#if row !== null}
            <button class="sha-link" onclick={() => onJumpToRow(row)}>
              {relativeLabel(parentSha)}
            </button>
          {:else}
            <span class="sha-link unloaded" title="Not loaded">
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
          <button class="sha-link" onclick={() => { const row = relativeRow(childSha); if (row !== null) onJumpToRow(row); }}>
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
        <FileTree nodes={fileTree} {diffFile} {onSelectFile}
                  onStage={node.kind === 'working' ? onStage : undefined} staging={writing} />
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
            {#if file.staging}
              <StageToggle {file} staged={file.staging.staged} disabled={writing} {onStage} />
            {/if}
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

  .chip-delete {
    margin-left: 4px;
    padding: 0 2px;
    font: inherit;
    line-height: 1;
    color: inherit;
    background: transparent;
    border: none;
    opacity: 0.6;
    cursor: pointer;
  }

  .chip-delete:hover {
    opacity: 1;
  }

  .tag-form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .tag-input {
    font: inherit;
    font-size: var(--font-size);
    color: var(--text);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 4px 6px;
    resize: vertical;
  }

  .tag-input:focus {
    outline: none;
    border-color: var(--accent-border-strong);
  }

  .tag-error {
    color: var(--error);
    font-size: 11px;
  }

  .tag-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .tag-btn {
    align-self: flex-start;
    margin-top: 8px;
    font: inherit;
    font-size: 11px;
    color: var(--text-muted);
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 2px 8px;
    cursor: pointer;
    transition: var(--transition);
  }

  .tag-actions .tag-btn {
    margin-top: 0;
  }

  .commit-actions {
    justify-content: flex-start;
    margin-top: 8px;
  }

  .switch-to {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .stash-actions {
    justify-content: flex-start;
    margin-top: 8px;
  }

  .tag-btn:hover:not(:disabled) {
    border-color: var(--border-hover);
    color: var(--text);
  }

  .tag-btn.primary {
    color: var(--accent);
    background: var(--accent-bg);
    border-color: var(--accent-border);
  }

  .tag-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

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
