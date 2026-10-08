<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import GraphCanvas, { EMPTY_GRAPH_WIDTH, REFS_WIDTH, graphAreaWidth } from './GraphCanvas.svelte';
  import DiffViewer from './DiffViewer.svelte';
  import SearchBox from './SearchBox.svelte';
  import CommitDetail from './CommitDetail.svelte';
  import RepoSidebar from './RepoSidebar.svelte';
  import StatusBar, { READY_STATUS } from './StatusBar.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import InProgress from './InProgress.svelte';
  import { refShas } from './refTree.ts';
  import { commitChips } from './refChips.ts';
  import type { RowRange } from './rowWindow.ts';
  import { Newest } from './hostAnswers.ts';
  import { actionLabel, confirmedWrite, type Outcome, type Write } from './writes.ts';
  import { canStop, fetchOf, pullOf, pushOf } from './transfers.ts';
  import { historyActions } from './actions.ts';
  import StashDialog from './StashDialog.svelte';
  import type {
    NodeJson, FileChange, DiffLine, RepoRefs, Growth, RowWindow, Relative, Selected, Found,
  } from './types.ts';
  import { applyTheme, storedTheme, type Theme } from './theme/palette.ts';

  // ── Theme ────────────────────────────────────────────────────────────────
  let theme = $state<Theme>(storedTheme());

  $effect(() => {
    applyTheme(theme);
  });

  function toggleTheme(): void {
    theme = theme === 'dark' ? 'light' : 'dark';
  }

  // ── Avatars — opt-in, default off (ADR-0023) ─────────────────────────────
  const storedAvatars = typeof localStorage !== 'undefined' ? localStorage.getItem('avatars') : null;
  let showAvatars = $state(storedAvatars === 'on');

  $effect(() => {
    try { localStorage.setItem('avatars', showAvatars ? 'on' : 'off'); } catch { /* ignore */ }
  });

  function toggleAvatars(): void {
    showAvatars = !showAvatars;
  }

  // ── State ──────────────────────────────────────────────────────────────────
  // The host keeps the history (ADR-0031); the web interface holds how far it has loaded, the
  // window of rows on screen, and the selected commit.
  let growth          = $state<Growth | null>(null);
  let view            = $state<RowWindow | null>(null);
  let repoRefs        = $state<RepoRefs | null>(null);
  let status          = $state(READY_STATUS);
  let repoLabel       = $state('(no repository)');
  let currentRepoPath = $state<string | null>(null);
  let selectedNode    = $state<NodeJson | null>(null);
  let relatives       = $state(new Map<string, Relative>());
  // The commits of the sidebar's refs that are laid out, so a ref to history still loading stays inert.
  let loadedRefShas   = $state(new Set<string>());
  let graphCanvas     = $state<{ revealRow: (row: number) => void; clearSelection: () => void } | null>(null);
  let searchBox       = $state<{ focusSearch: () => void } | null>(null);
  const rowCount      = $derived(growth?.rows ?? 0);
  // The most lanes the graph has shown in this load: the graph column grows to them, so older
  // history arriving with more lanes moves nothing until it is scrolled to.
  let shownLanes      = $state(0);
  // The checked-out branch, taken from the refs payload so the graph can mark
  // its chip. Detached HEAD leaves it null and nothing is marked.
  const headRef = $derived(repoRefs?.locals.find(b => b.isHead)?.name ?? null);

  // Width of the graph lane area — the column header mirrors the canvas layout.
  const graphColWidth = $derived(rowCount === 0 ? EMPTY_GRAPH_WIDTH : graphAreaWidth(Math.max(shownLanes, 1) - 1));

  // ── Rows from the host ─────────────────────────────────────────────────────
  /** The rows last asked for, asked again whenever the history or the selection changes. */
  let wanted: RowRange | null = null;
  // The newest answer of each kind for the current load; older ones are dropped (ADR-0031).
  const reads = new Newest();
  const selects = new Newest();
  const refreshes = new Newest();
  const currentLoad = (): number | undefined => growth?.generation;
  // The question open before a write that discards work, and how it is answered (ADR-0034).
  let question = $state<
    { lost: string; action: string; answer: (confirmed: boolean) => void } | null
  >(null);
  // What the status bar says over the load's own line until the developer acts again: why a
  // transfer was refused, which the reload after it would otherwise hide.
  let notice = $state<string | null>(null);
  // Whether the dialog that describes a new stash is open.
  let stashing = $state(false);
  // Whether a transfer to or from a remote runs, and whether the toolbar's button can cancel it,
  // as the core decides (ADR-0035).
  let fetching = $state(false);
  let cancellable = $state(false);
  // The fetch, pull, and push the refs make possible, or why there is none, as the core decides.
  const fetchable = $derived(repoRefs ? fetchOf(repoRefs) : { reason: 'No repository is open' });
  const pullable = $derived(repoRefs ? pullOf(repoRefs) : { reason: 'No repository is open' });
  const pushable = $derived(repoRefs ? pushOf(repoRefs) : { reason: 'No repository is open' });

  /** Runs a transfer, the toolbar's buttons turning into one that cancels it. A transfer refused
      may have moved remote branches by its fetch, so the repository is read again, the refusal
      kept in view (ADR-0035). */
  async function transfer(answer: { write: Write } | { reason: string }, doing: string): Promise<void> {
    if (!('write' in answer)) {
      status = answer.reason;
      return;
    }
    fetching = true;
    cancellable = canStop(answer.write);
    status = doing;
    const error = await performWrite(answer.write);
    fetching = false;
    if (error) {
      await reloadAfterWrite(null);
      notice = 'Error: ' + error;
    }
  }

  /** Fetches the default remote; the reload after shows its branches. */
  async function fetchRemote(): Promise<void> {
    await transfer(fetchable, 'write' in fetchable ? `Fetching ${fetchable.write.remote}…` : '');
  }

  /** Pushes the checked-out branch to its upstream, or sets one; never by force. */
  async function pushBranch(): Promise<void> {
    await transfer(pushable, 'write' in pushable ? `Pushing ${pushable.write.branch}…` : '');
  }

  /** Pulls the checked-out branch from its upstream, fast-forward only. */
  async function pullBranch(): Promise<void> {
    await transfer(pullable, 'write' in pullable ? `Pulling ${pullable.write.branch}…` : '');
  }
  // The selected commit's ref chips, each with the switch or delete it offers.
  const selectedChips = $derived(
    selectedNode ? commitChips(selectedNode.refs, repoRefs, selectedNode.sha) : [],
  );
  // The operations of G-5 the selected commit offers on the checked-out branch (ADR-0033).
  const selectedActions = $derived.by(() => {
    if (!repoRefs || selectedNode?.kind !== 'commit') return [];
    const offered = historyActions(repoRefs, selectedNode.sha);
    return 'actions' in offered ? offered.actions : [];
  });

  /** A command that failed: the status bar says so. */
  function report(e: unknown): void {
    status = 'Error: ' + e;
  }

  async function readWindow(range: RowRange): Promise<void> {
    wanted = range;
    try {
      const window = await reads.ask(() => invoke<RowWindow>('read_window', { ...range }), currentLoad);
      if (window) {
        view = window;
        shownLanes = Math.max(shownLanes, window.lanes);
      }
    } catch (e) {
      report(e);
    }
  }

  function rereadWindow(): void {
    if (wanted) void readWindow(wanted);
  }

  async function updateLoadedRefs(): Promise<void> {
    const shas = repoRefs ? refShas(repoRefs) : [];
    try {
      const rows = await invoke<(number | null)[]>('rows_of', { shas });
      loadedRefShas = new Set(shas.filter((_, i) => rows[i] !== null && rows[i] !== undefined));
    } catch (e) {
      report(e);
    }
  }

  function onGrowth(next: Growth): void {
    if (next.generation !== growth?.generation) {
      // A new load: rows are reassigned, so a selection or a window from the history before
      // would point at the wrong commits.
      view = null;
      wanted = null;
      shownLanes = 0;
      reads.drop();
      selects.drop();
      refreshes.drop();
      selectedNode = null;
      relatives = new Map();
      changedFiles = [];
      diffFile = null;
      diffLines = [];
    }
    growth = next;
    rereadWindow();
    void updateLoadedRefs();
    // The selection's parents and lineage may reach into the rows just added.
    if (selectedNode) void refreshRelatives(selectedNode.sha);
    // The commit a write left selected, which the core names once this load has laid it out.
    if (next.select !== null) jumpToRow(next.select);
  }

  async function refreshRelatives(sha: string): Promise<void> {
    try {
      const selected = await refreshes.ask(() => invoke<Selected | null>('selection'), currentLoad);
      if (selected && selected.node.sha === sha && selectedNode?.sha === sha) {
        relatives = relativesOf(selected);
      }
    } catch (e) {
      report(e);
    }
  }

  function relativesOf(selected: Selected): Map<string, Relative> {
    return new Map(selected.relatives.map(r => [r.sha, r]));
  }

  async function searchHistory(query: string): Promise<Found> {
    const empty: Found = { generation: growth?.generation ?? 0, rows: [], hits: [] };
    try {
      const found = await invoke<Found>('search_history', { query });
      return found.generation === growth?.generation ? found : empty;
    } catch (e) {
      report(e);
      return empty;
    }
  }
  let changedFiles    = $state<FileChange[]>([]);

  let diffLines       = $state<DiffLine[]>([]);
  let diffFile        = $state<string | null>(null);
  let diffLoading     = $state(false);

  // ── Keyboard shortcuts ─────────────────────────────────────────────────────

  /** True when the key event targets a text-editing element — those keep
      their native key handling and are excluded from graph shortcuts. */
  function isEditableTarget(e: KeyboardEvent): boolean {
    const t = e.target as HTMLElement | null;
    return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
  }

  /** Escape: deselect the commit, which also lifts the ancestry dim. */
  function clearSelection(): void {
    selects.drop();
    refreshes.drop();
    selectedNode = null;
    relatives = new Map();
    invoke('deselect').then(rereadWindow, report);
    changedFiles = [];
    diffFile = null;
    diffLines = [];
    graphCanvas?.clearSelection();
  }

  /** ↑/↓: step the selection one row; ↓ with nothing selected starts at row 0. */
  function moveSelection(delta: 1 | -1): void {
    const row = selectedNode ? selectedNode.row + delta : 0;
    if (row < 0 || row >= rowCount) return;
    jumpToRow(row);
  }

  /** Global shortcuts: Ctrl/Cmd+F focuses the search field; Escape closes the
      diff overlay, then clears the selection; ↑/↓ walk the commit rows. */
  function onWindowKeydown(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
      e.preventDefault();
      searchBox?.focusSearch();
      return;
    }
    if (isEditableTarget(e)) return;
    if (e.key === 'Escape') {
      if (diffFile !== null) closeDiff();
      else clearSelection();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      moveSelection(1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      moveSelection(-1);
    }
  }

  // ── Tauri event listeners ──────────────────────────────────────────────────
  onMount(() => {
    const unlisten = Promise.all([
      listen<Growth>('history-grew', (e) => onGrowth(e.payload)),
      // Refs come from the same repository read as the graph (ADR-0026), so the
      // sidebar never describes a different state than the commits on screen.
      listen<string>('load-refs', (e) => {
        try {
          repoRefs = typeof e.payload === 'string' ? JSON.parse(e.payload) : e.payload;
        } catch {
          repoRefs = null;
        }
        void updateLoadedRefs();
      }),
      listen<string>('set-status',     (e) => { status = e.payload; }),
      listen<string>('write-progress', (e) => { if (fetching) status = e.payload; }),
      listen<string>('commit-files', (e) => {
        try {
          changedFiles = typeof e.payload === 'string' ? JSON.parse(e.payload) : e.payload;
        } catch { changedFiles = []; }
      }),
      listen<string>('set-repo-label', (e) => {
        repoLabel       = e.payload || '(no repository)';
        currentRepoPath = e.payload;
      }),
    ]);
    // A repository named on the command line opens once every listener stands, the same way a
    // path from the sidebar does, so its errors reach the status bar like any other.
    unlisten.then(async () => {
      const path = await invoke<string | null>('startup_repo').catch((e: unknown) => {
        status = 'Error: ' + e;
        return null;
      });
      if (path) await openRepoPath(path);
    });
    return () => { unlisten.then(fns => fns.forEach(fn => fn())); };
  });

  // ── Actions ────────────────────────────────────────────────────────────────
  async function browseRepo(): Promise<void> {
    notice = null;
    status = 'Selecting repository…';
    await invoke('browse_repo').catch((e: unknown) => { status = 'Error: ' + e; });
  }

  /** Loads the current repository again; `select` names a commit for the core to select again
      once the load lays it out (ADR-0033). */
  async function refreshRepo(select: string | null = null): Promise<void> {
    if (!currentRepoPath) return;
    notice = null;
    status = 'Refreshing…';
    await invoke('open_repo', { path: currentRepoPath, select }).catch((e: unknown) => {
      status = 'Error: ' + e;
    });
  }

  /** Loads again after a write, selecting the commit it made, else the one selected now: the
      developer may have selected another while the write ran. */
  async function reloadAfterWrite(made: string | null): Promise<void> {
    await refreshRepo(made ?? selectedNode?.sha ?? null);
  }

  /// Switches to another repository the sidebar points at — a submodule, a
  /// working tree, or the parent module (ADR-0027). It is the same load the
  /// folder picker triggers, so the graph, the refs and the label all follow.
  async function openRepoPath(path: string): Promise<void> {
    notice = null;
    if (!path) return;
    status = 'Opening…';
    await invoke('open_repo', { path }).catch((e: unknown) => { status = 'Error: ' + e; });
  }

  /** Performs a write (G-7), asking first where the core says it discards work: the reload
      after it shows the result, the commit selected again. Resolves with why the host refused
      it, or null once written or kept. */
  async function performWrite(write: Write): Promise<string | null> {
    // One write at a time, from whichever panel asks for it.
    if (writeRunning) return 'A write is still running';
    writeRunning = true;
    try {
      return await performOneWrite(write);
    } finally {
      writeRunning = false;
    }
  }

  let writeRunning = false;

  async function performOneWrite(write: Write): Promise<string | null> {
    notice = null;
    let done: Outcome;
    try {
      done = await confirmedWrite(
        write,
        (w) => invoke<string | null>('write_discards', { write: w }),
        ask,
        (w, confirmed) => invoke<string | null>('perform_write', { write: w, confirmed }),
      );
    } catch (e) {
      return String(e);
    }
    if (done.kind === 'written') await reloadAfterWrite(done.made);
    return null;
  }

  /** Puts the question before a write that discards work; resolves with the answer. */
  function ask(lost: string, write: Write): Promise<boolean> {
    // A question still open is answered no: only one stands at a time.
    question?.answer(false);
    return new Promise(resolve => {
      question = {
        lost,
        action: actionLabel(write),
        answer: (confirmed) => {
          question = null;
          resolve(confirmed);
        },
      };
    });
  }

  /** Selects the commit in `row`: the host computes its lineage, which the window read after
      lights up, and its changed files follow. */
  async function onSelectRow(row: number): Promise<void> {
    notice = null;
    let selected: Selected | null;
    try {
      selected = await selects.ask(() => invoke<Selected | null>('select_row', { row }), currentLoad);
    } catch (e) {
      report(e);
      return;
    }
    if (!selected) return;
    refreshes.drop();
    selectedNode = selected.node;
    relatives = relativesOf(selected);
    changedFiles = [];
    diffFile = null;
    diffLines = [];
    rereadWindow();
    invoke('select_commit', { sha: selected.node.sha }).catch(report);
  }

  /** Jump to a row from a parent/child link, a search hit, or the arrow keys. */
  function jumpToRow(row: number): void {
    graphCanvas?.revealRow(row);
    void onSelectRow(row);
  }

  /** Jump to the commit a sidebar ref points at, once it is laid out. */
  async function jumpToSha(sha: string): Promise<void> {
    try {
      const [row] = await invoke<(number | null)[]>('rows_of', { shas: [sha] });
      if (row !== null && row !== undefined) jumpToRow(row);
    } catch (e) {
      report(e);
    }
  }

  async function onSelectFile(file: FileChange): Promise<void> {
    if (!selectedNode) return;
    diffFile = file.path;
    diffLines = [];
    diffLoading = true;
    try {
      const json = await invoke<string>('get_file_diff', { sha: selectedNode.sha, filePath: file.path });
      diffLines = typeof json === 'string' ? JSON.parse(json) : json;
    } catch (e) {
      diffLines = [];
      report(e);
    } finally {
      diffLoading = false;
    }
  }

  function closeDiff(): void {
    diffFile = null;
    diffLines = [];
  }
</script>

<!-- ── Titlebar / Toolbar ─────────────────────────────────────────────────── -->
<div id="titlebar">
  <div id="titlebar-left">
    <svg class="logo-icon" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="10" stroke="#4caf7d" stroke-width="2"/>
      <circle cx="12" cy="8"  r="2"  fill="#4caf7d"/>
      <circle cx="7"  cy="14" r="2"  fill="#89b4fa"/>
      <circle cx="17" cy="14" r="2"  fill="#fab387"/>
      <line x1="12" y1="10" x2="7"  y2="14" stroke="#4caf7d" stroke-width="1.5"/>
      <line x1="12" y1="10" x2="17" y2="14" stroke="#4caf7d" stroke-width="1.5"/>
    </svg>
    <span class="app-name">Ocelli</span>
  </div>

  <div id="titlebar-center">
    <button class="tb-btn primary" onclick={browseRepo}>
      <svg viewBox="0 0 16 16" fill="currentColor">
        <path d="M1 3.5A1.5 1.5 0 012.5 2h3.672a1.5 1.5 0 011.06.44l.94.94H13.5A1.5 1.5 0 0115 5v7.5a1.5 1.5 0 01-1.5 1.5h-11A1.5 1.5 0 011 12.5v-9z"/>
      </svg>
      Open Repo
    </button>
    <button class="tb-btn" onclick={() => refreshRepo()}>
      <svg viewBox="0 0 16 16" fill="currentColor">
        <path d="M11.534 7h3.932a.25.25 0 01.192.41l-1.966 2.36a.25.25 0 01-.384 0l-1.966-2.36a.25.25 0 01.192-.41zm-11 2h3.932a.25.25 0 00.192-.41L2.692 6.23a.25.25 0 00-.384 0L.342 8.59A.25.25 0 00.534 9z"/>
        <path fill-rule="evenodd" d="M8 3c-1.552 0-2.94.707-3.857 1.818a.5.5 0 11-.771-.636A6.002 6.002 0 0113.917 7H12.9A5.002 5.002 0 008 3zM3.1 9a5.002 5.002 0 008.757 2.182.5.5 0 11.771.636A6.002 6.002 0 012.083 9H3.1z"/>
      </svg>
      Refresh
    </button>
    <button class="tb-btn" onclick={() => (stashing = true)} disabled={!currentRepoPath}
            title="Put the uncommitted changes aside as a stash">
      Stash…
    </button>
    {#if fetching}
      <button class="tb-btn" disabled={!cancellable}
              onclick={() => invoke('cancel_write').catch(report)}>
        {cancellable ? 'Cancel' : 'Pushing…'}
      </button>
    {:else}
      <button class="tb-btn" onclick={fetchRemote} disabled={!('write' in fetchable)}
              title={'write' in fetchable ? `Fetch ${fetchable.write.remote}` : fetchable.reason}>
        Fetch
      </button>
      <button class="tb-btn" onclick={pullBranch} disabled={!('write' in pullable)}
              title={'write' in pullable
                ? `Pull ${pullable.write.branch} from ${repoRefs?.locals.find(b => b.isHead)?.upstream}, fast-forward only`
                : pullable.reason}>
        Pull
      </button>
      <button class="tb-btn" onclick={pushBranch} disabled={!('write' in pushable)}
              title={'write' in pushable ? `Push ${pushable.write.branch}` : pushable.reason}>
        Push
      </button>
    {/if}

    <div class="repo-breadcrumb">
      <svg viewBox="0 0 16 16" fill="#4caf7d" class="repo-icon">
        <path d="M2.6 10.59L8 5.207l5.4 5.38.8-.795-6.2-6.187-6.2 6.187.8.795z"/>
      </svg>
      <span class="repo-path">{repoLabel}</span>
    </div>
  </div>

  <div id="titlebar-right">
    <SearchBox bind:this={searchBox} search={searchHistory} revision={rowCount} onJump={jumpToRow} />
  </div>
</div>

<svelte:window onkeydown={onWindowKeydown} />

<!-- ── Main content area ──────────────────────────────────────────────────── -->
{#if repoRefs?.inProgress}
  <InProgress stopped={repoRefs.inProgress} onWrite={performWrite} />
{/if}
<div id="main">
  <div id="top-pane">
    <RepoSidebar
      {repoRefs}
      loadedShas={loadedRefShas}
      onJump={jumpToSha}
      onOpenRepo={openRepoPath}
    />

    <div id="graph-pane">
      <!-- Column header — inside the graph pane so it shares the canvas width,
           with the graph column tracking the canvas's dynamic lane width. -->
      <div id="col-header">
        <span class="col-refs" style="width: {REFS_WIDTH}px">Branch / Tag</span>
        <span class="col-graph" style="width: {graphColWidth + 8}px">Graph</span>
        <span class="col-sha">SHA</span>
        <span class="col-desc">Description</span>
      </div>

      <div id="canvas-container">
        <GraphCanvas
          bind:this={graphCanvas}
          {rowCount}
          lanes={Math.max(shownLanes, 1)}
          {view}
          generation={growth?.generation ?? 0}
          onNeedRows={readWindow}
          {onSelectRow}
          {theme}
          {headRef}
        />

        {#if diffFile !== null}
          <div id="diff-overlay">
            {#if diffLoading}
              <div class="diff-loading">
                <div class="spinner"></div>
                <span>Loading diff…</span>
              </div>
            {:else}
              <DiffViewer lines={diffLines} filePath={diffFile} onClose={closeDiff} />
            {/if}
          </div>
        {/if}
      </div>
    </div><!-- end #graph-pane -->

    <CommitDetail
      node={selectedNode}
      {relatives}
      {showAvatars}
      {changedFiles}
      {diffFile}
      {onSelectFile}
      onJumpToRow={jumpToRow}
      chips={selectedChips}
      actions={selectedActions}
      onWrite={performWrite}
    />
  </div><!-- end #top-pane -->
</div><!-- end #main -->

{#if stashing}
  <StashDialog onStash={performWrite} onClose={() => (stashing = false)} />
{/if}

{#if question}
  <ConfirmDialog lost={question.lost} action={question.action} onAnswer={question.answer} />
{/if}

<StatusBar
  status={notice ?? status}
  {theme}
  {showAvatars}
  onToggleTheme={toggleTheme}
  onToggleAvatars={toggleAvatars}
/>

<style>
  /* Layout of the app shell; its colors and typography come from theme.css. */
  :global(body) {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }

  /* ── Titlebar ────────────────────────────────────────────────────────────── */
  #titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 12px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    gap: 12px;
  }

  #titlebar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .logo-icon {
    width: 22px;
    height: 22px;
  }

  .app-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-brightest);
    letter-spacing: 0.02em;
  }

  #titlebar-center {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
  }

  #titlebar-right {
    flex-shrink: 0;
    display: flex;
    justify-content: flex-end;
  }

  .tb-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    background: var(--bg-elev);
    color: var(--text-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
    transition: background var(--transition), border-color var(--transition);
    flex-shrink: 0;
  }

  .tb-btn svg {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  .tb-btn:hover {
    background: var(--btn-hover-bg);
    border-color: var(--border-hover);
    color: var(--text-bright);
  }

  .tb-btn.primary {
    background: var(--accent-bg-strong);
    border-color: var(--accent-border-strong);
    color: var(--accent);
  }

  .tb-btn.primary:hover {
    background: var(--accent-bg-strong-hover);
    border-color: var(--accent-border-strong-hover);
    color: var(--accent-bright);
  }

  .repo-breadcrumb {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-left: 8px;
    padding: 4px 10px;
    background: var(--bg-hover);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    max-width: 420px;
  }

  .repo-icon {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
  }

  .repo-path {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ── Column header ───────────────────────────────────────────────────────── */
  #col-header {
    display: flex;
    align-items: center;
    height: 24px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
    font-size: 10px;
    font-weight: 600;
    color: var(--text-dim);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    flex-shrink: 0;
  }

  /* Column starts mirror the canvas layout in GraphCanvas.svelte: branch/tag
     chips first, then the graph lanes, then sha at refs-width + lane-width + 8,
     with the description taking the rest of the row. Both leading columns take
     their width inline — the refs column from the shared constant, the graph
     column because its lane area grows with the graph. The refs header is
     right-aligned because the chips below it hug the graph lanes. */
  .col-refs  { flex-shrink: 0; padding-right: 8px; text-align: right; }
  .col-graph { flex-shrink: 0; padding-left: 8px; }
  .col-sha   { width: 80px; flex-shrink: 0; font-family: var(--font-mono); }
  .col-desc  { flex: 1;     min-width: 0; padding-right: 8px; }

  /* ── Main layout ─────────────────────────────────────────────────────────── */
  #main {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  #top-pane {
    display: flex;
    flex: 1;
    overflow: hidden;
    min-height: 0;
  }

  #graph-pane {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  #canvas-container {
    flex: 1;
    overflow: hidden;
    position: relative;
    min-height: 0;
  }

  #diff-overlay {
    position: absolute;
    inset: 0; /* cover the commit-tree canvas */
    background: var(--bg-panel);
    box-shadow: 0 0 32px var(--shadow);
    z-index: 100;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .diff-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    height: 100%;
    color: var(--text-dimmer);
    font-size: 12px;
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
</style>
