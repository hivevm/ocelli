<script lang="ts">
  // The repository sidebar (ADR-0026, ADR-0027): local branches, remote branches
  // grouped by remote, working trees, tags, and submodules. It navigates and
  // nothing else: the writes on a ref are the ref chips' (ADR-0033).
  //
  // Rows come in two kinds. Branches and tags *reveal* a commit in the graph.
  // Working trees, submodules and the parent row *open* a repository, because
  // what they point at is a different checkout or a different object database
  // (ADR-0027) — switching is a full reload, so those rows say where they lead.
  import { anyLeaf, countLeaves, filterByName, groupByFolder } from './refTree.ts';
  import type { RefFolder } from './refTree.ts';
  import { parentBlocked, submoduleBlocked, worktreeBlocked } from './repoSwitch.ts';
  import type { RepoRefs } from './types.ts';

  interface Props {
    repoRefs: RepoRefs | null;
    /** SHAs present in the loaded graph; a ref outside it has nowhere to jump to. */
    loadedShas: Set<string>;
    onJump: (sha: string) => void;
    /** Loads another repository — a submodule, a working tree, or the parent. */
    onOpenRepo: (path: string) => void;
  }
  let { repoRefs, loadedShas, onJump, onOpenRepo }: Props = $props();

  // Glyphs as bare path data: the collapsed rail shows nothing but these, so
  // every one of them has to be recognisable without its label next to it.
  const ICON_BRANCH =
    'M11.5 2a2.5 2.5 0 00-.86 4.85A2.5 2.5 0 018.15 9H6.5a2.5 2.5 0 00-1.5.5V6.35a2.5 2.5 0 10-1 0v3.3a2.5 2.5 0 101 .17A1.5 1.5 0 016.5 10h1.65a3.5 3.5 0 003.48-3.15A2.5 2.5 0 0011.5 2z';
  const ICON_CLOUD = 'M4.5 12a3.5 3.5 0 01-.36-6.98A4.5 4.5 0 0112.9 6.1 3 3 0 0112.5 12h-8z';
  const ICON_TREE =
    'M1.5 3A1.5 1.5 0 013 1.5h10A1.5 1.5 0 0114.5 3v7a1.5 1.5 0 01-1.5 1.5H9.5V13h2v1h-7v-1h2v-1.5H3A1.5 1.5 0 011.5 10V3z';
  const ICON_TAG =
    'M2 2h5.2a1.5 1.5 0 011.06.44l5.3 5.3a1.5 1.5 0 010 2.12l-3.7 3.7a1.5 1.5 0 01-2.12 0l-5.3-5.3A1.5 1.5 0 012 7.2V2zm2.75 1.5a1.25 1.25 0 100 2.5 1.25 1.25 0 000-2.5z';
  const ICON_FOLDER =
    'M1.5 3.5A1.5 1.5 0 013 2h3l1.5 1.5H13A1.5 1.5 0 0114.5 5v6.5A1.5 1.5 0 0113 13H3a1.5 1.5 0 01-1.5-1.5v-8z';
  // A package: a repository nested inside this one, pinned to one commit.
  const ICON_SUBMODULE =
    'M8 1.2l6 3.3v6.9l-6 3.3-6-3.3V4.5l6-3.3zm0 1.6L3.6 5.2 8 7.6l4.4-2.4L8 2.8zM3 6.3v4.6l4.5 2.5V8.8L3 6.3zm10 0L8.5 8.8v4.6L13 10.9V6.3z';
  const ICON_UP = 'M8 2.2l4.5 4.5-1.4 1.4L9 6v7.8H7V6L4.9 8.1 3.5 6.7 8 2.2z';
  // The checked-out branch, and the working tree being viewed: a tick in a ring,
  // so "you are here" is a shape and not only a colour.
  const ICON_CHECK =
    'M8 1a7 7 0 100 14A7 7 0 008 1zm3.6 4.9l-4.2 4.9a.9.9 0 01-1.3.1L3.9 8.9l1.2-1.4 1.5 1.3 3.6-4.2 1.4 1.3z';

  /** One row of the sidebar, whatever ref category it came from. */
  interface Entry {
    /** Full slash-separated name — what grouping and filtering work on. */
    name: string;
    sha: string;
    shortSha: string;
    /** Marks the checked-out branch. */
    isHead: boolean;
    /** Trailing annotation: ahead/behind counts, or a worktree's branch. */
    badge: string;
    /** Tooltip: the upstream, or a worktree's path. */
    title: string;
    /** Glyph path, so a row reads as its category without its section in view. */
    icon: string;
    /** "You are here" in words: the checked-out branch, the working tree on screen. */
    mark?: string;
    /** When set, selecting the row opens that repository instead of revealing a commit. */
    openPath?: string;
    /** Why the row cannot act — shown instead of the normal tooltip. */
    blocked?: string;
  }

  function entry(name: string, sha: string, shortSha: string, rest: Partial<Entry> = {}): Entry {
    return { name, sha, shortSha, isHead: false, badge: '', title: '', icon: ICON_BRANCH, ...rest };
  }

  // Ahead/behind is only meaningful where there is an upstream, and a branch
  // level with it earns no badge at all — zeros would be noise on every row.
  function trackingBadge(ahead: number, behind: number): string {
    const parts: string[] = [];
    if (ahead > 0) parts.push(`${ahead}↑`);
    if (behind > 0) parts.push(`${behind}↓`);
    return parts.join(' ');
  }

  const locals = $derived(
    (repoRefs?.locals ?? []).map(b =>
      entry(b.name, b.sha, b.shortSha, {
        isHead: b.isHead,
        // The one branch `HEAD` points at, said in words next to the name. The
        // tracking counts stay their own badge — they answer a different
        // question, and a branch can be checked out and ahead at once.
        mark: b.isHead ? 'checked out' : undefined,
        badge: trackingBadge(b.ahead, b.behind),
        title: b.upstream ? `tracks ${b.upstream}` : 'no upstream configured',
      }),
    ),
  );

  const remotes = $derived(
    (repoRefs?.remotes ?? []).map(remote => ({
      name: remote.name,
      entries: remote.branches.map(b => entry(b.name, b.sha, b.shortSha)),
    })),
  );

  // A working tree row switches to that checkout: the history is the same
  // object database, but HEAD, the checked-out branch and the working-directory
  // node (ADR-0024) are that tree's own. The one being viewed is marked and
  // inert, like a branch that is already HEAD.
  const worktrees = $derived(
    (repoRefs?.worktrees ?? []).map(w =>
      entry(w.name, w.sha, w.shortSha, {
        isHead: w.isCurrent,
        mark: w.isCurrent ? 'viewing' : undefined,
        badge: w.branch ?? 'detached',
        title: w.path,
        icon: ICON_TREE,
        openPath: w.path,
        blocked: worktreeBlocked(w),
      }),
    ),
  );

  // A submodule's commit belongs to *its* object database, so the row opens the
  // repository instead of jumping (ADR-0027). Grouping runs on the path, so
  // `vendor/a` and `vendor/b` fold into a folder like any other ref name.
  const submodules = $derived(
    (repoRefs?.submodules ?? []).map(sub =>
      entry(sub.path, sub.sha, sub.shortSha, {
        badge: sub.state === 'modified' ? 'modified' : sub.state === 'uninitialized' ? 'not initialized' : '',
        title: [sub.url, sub.state === 'modified' ? `checked out at ${sub.checkedOutShortSha}` : '']
          .filter(Boolean)
          .join(' — '),
        icon: ICON_SUBMODULE,
        openPath: sub.workdir,
        blocked: submoduleBlocked(sub),
      }),
    ),
  );

  const parent = $derived(repoRefs?.parent ?? null);
  // A parent outside the remote server's root shows without its switch, saying why (ADR-0039).
  const parentReason = $derived(parent ? parentBlocked(parent) : undefined);

  const tags = $derived(
    (repoRefs?.tags ?? []).map(t => entry(t.name, t.sha, t.shortSha, { icon: ICON_TAG })),
  );

  // ── Filter ────────────────────────────────────────────────────────────────
  let filter = $state('');

  function visible(entries: Entry[]): Entry[] {
    return filterByName(entries, filter);
  }

  function tree(entries: Entry[]): RefFolder<Entry> {
    return groupByFolder(visible(entries));
  }

  // ── Sections ──────────────────────────────────────────────────────────────
  // Titles, glyphs and counts in one place: the expanded headers and the
  // collapsed rail render the same four sections and must not drift apart.
  const sections = $derived([
    { key: 'local', title: 'Local', icon: ICON_BRANCH, count: visible(locals).length },
    {
      key: 'remote',
      title: 'Remote',
      icon: ICON_CLOUD,
      count: remotes.reduce((n, r) => n + visible(r.entries).length, 0),
    },
    { key: 'worktrees', title: 'Worktrees', icon: ICON_TREE, count: visible(worktrees).length },
    { key: 'tags', title: 'Tags', icon: ICON_TAG, count: visible(tags).length },
    {
      key: 'submodules',
      title: 'Submodules',
      icon: ICON_SUBMODULE,
      count: visible(submodules).length,
    },
  ]);

  // ── Minimised state ───────────────────────────────────────────────────────
  // Minimising trades the lists for a rail of section glyphs, so the graph gets
  // the width back without losing the way back in. Remembered across sessions,
  // like the diff viewer's layout choice.
  const storedCollapsed =
    typeof localStorage !== 'undefined' ? localStorage.getItem('sidebar') : null;
  let collapsed = $state(storedCollapsed === 'collapsed');

  $effect(() => {
    try {
      localStorage.setItem('sidebar', collapsed ? 'collapsed' : 'expanded');
    } catch {
      /* ignore */
    }
  });

  function toggleCollapsed(): void {
    collapsed = !collapsed;
    // The filter field goes away with the lists, and a filter nobody can see
    // must not silently thin the rail's counts.
    if (collapsed) filter = '';
  }

  /** From the rail, a glyph is the only way back — so it expands *and* opens. */
  function openFromRail(key: string): void {
    collapsed = false;
    openSections[key] = true;
  }

  // ── Collapse state ────────────────────────────────────────────────────────
  // Local branches are what a user reaches for first, so that section starts
  // open and the rest stay out of the way until asked for.
  let openSections = $state<Record<string, boolean>>({
    local: true,
    remote: false,
    worktrees: false,
    tags: false,
    submodules: false,
  });

  // Folders and remotes are open unless explicitly closed, so a fresh repository
  // shows its structure rather than a wall of collapsed rows.
  let closed = $state<Record<string, boolean>>({});

  function toggleSection(key: string): void {
    openSections[key] = !openSections[key];
  }

  function toggleGroup(key: string): void {
    closed[key] = !closed[key];
  }

  /** A filter hides the collapse state: every match should be visible at once. */
  function groupOpen(key: string): boolean {
    return filter.trim() !== '' || !closed[key];
  }

  function canJump(sha: string): boolean {
    return sha !== '' && loadedShas.has(sha);
  }

  /// Whether a row can act at all: a repository-switching row needs a path and
  /// no blocking reason, every other row needs its commit in the loaded graph.
  function canAct(item: Entry): boolean {
    if (item.openPath !== undefined) return item.blocked === undefined && item.openPath !== '';
    return canJump(item.sha);
  }

  /// The two row kinds part here: open a repository, or reveal a commit.
  function activate(item: Entry): void {
    if (!canAct(item)) return;
    if (item.openPath !== undefined) onOpenRepo(item.openPath);
    else onJump(item.sha);
  }

  function rowTitle(item: Entry): string {
    if (item.blocked) return `${item.name} — ${item.blocked}`;
    if (item.openPath !== undefined) {
      return [`${item.name} — open this repository`, item.title].filter(Boolean).join(' — ');
    }
    if (!canJump(item.sha)) return `${item.name} — outside the loaded history`;
    return [item.name, item.title].filter(Boolean).join(' — ');
  }
</script>

<!-- A leaf row. Rows that cannot act stay inert and say why: a ref can point at
     history that is not loaded (ADR-0016), a submodule may
     never have been checked out, and a working tree may be the one on screen.
     A button that silently does nothing is worse than one that explains. -->
{#snippet leafRow(label: string, item: Entry, depth: number)}
  {@const enabled = canAct(item)}
  <button
    class="row leaf"
    class:head={item.isHead}
    class:unreachable={!enabled}
    style="padding-left: {8 + depth * 12}px"
    disabled={!enabled}
    title={rowTitle(item)}
    onclick={() => activate(item)}
  >
    <svg class="glyph" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d={item.isHead ? ICON_CHECK : item.icon} />
    </svg>
    <span class="label">{label}</span>
    {#if item.mark}<span class="mark">{item.mark}</span>{/if}
    {#if item.badge}<span class="badge">{item.badge}</span>{/if}
    <span class="sha">{item.shortSha}</span>
  </button>
{/snippet}

<!-- Recursive: a folder renders its subfolders, then its own entries. -->
{#snippet folderTree(folder: RefFolder<Entry>, keyPrefix: string, depth: number)}
  {#each folder.folders as sub (sub.path)}
    {@const key = `${keyPrefix}/${sub.path}`}
    <button
      class="row folder"
      class:holds-head={anyLeaf(sub, e => e.isHead)}
      style="padding-left: {8 + depth * 12}px"
      onclick={() => toggleGroup(key)}
      title={anyLeaf(sub, e => e.isHead) ? `${sub.name} — holds the checked-out branch` : sub.name}
    >
      <span class="caret" class:open={groupOpen(key)}>›</span>
      <svg class="glyph" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
        <path d={ICON_FOLDER} />
      </svg>
      <span class="label">{sub.name}</span>
      <span class="count">{countLeaves(sub)}</span>
    </button>
    {#if groupOpen(key)}
      {@render folderTree(sub, key, depth + 1)}
    {/if}
  {/each}

  {#each folder.leaves as leaf (leaf.item.name)}
    {@render leafRow(leaf.label, leaf.item, depth)}
  {/each}
{/snippet}

{#snippet section(key: string)}
  {@const meta = sections.find(s => s.key === key)!}
  <button class="row section" onclick={() => toggleSection(key)}>
    <span class="caret" class:open={openSections[key]}>›</span>
    <svg class="glyph" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d={meta.icon} />
    </svg>
    <span class="label">{meta.title}</span>
    <span class="count">{meta.count}</span>
  </button>
{/snippet}

<aside id="repo-sidebar" class:collapsed>
  <div class="bar">
    {#if !collapsed}
      <div class="filter">
        <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
          <path
            fill-rule="evenodd"
            d="M11.5 7a4.5 4.5 0 11-9 0 4.5 4.5 0 019 0zm-.82 4.74a6 6 0 111.06-1.06l2.79 2.79-1.06 1.06-2.79-2.79z"
          />
        </svg>
        <input type="text" placeholder="Filter" bind:value={filter} aria-label="Filter refs" />
      </div>
    {/if}
    <button
      class="collapse-btn"
      onclick={toggleCollapsed}
      title={collapsed ? 'Expand the sidebar' : 'Minimise the sidebar'}
      aria-label={collapsed ? 'Expand the sidebar' : 'Minimise the sidebar'}
      aria-expanded={!collapsed}
    >
      <span class="chevron" class:flipped={collapsed}>‹</span>
    </button>
  </div>

  {#if collapsed}
    <!-- Minimised: glyphs only. Each is a way back in — it expands the sidebar
         and opens its own section, so the rail is never a dead end. -->
    <div class="rail">
      {#if parent}
        <button
          class="rail-btn"
          onclick={() => onOpenRepo(parent.path)}
          disabled={parentReason !== undefined}
          title={parentReason ? `${parent.name} — ${parentReason}` : `Back to ${parent.name}`}
          aria-label="Back to {parent.name}"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d={ICON_UP} />
          </svg>
        </button>
      {/if}
      {#each sections as item (item.key)}
        <button
          class="rail-btn"
          onclick={() => openFromRail(item.key)}
          title="{item.title} ({item.count})"
          aria-label="{item.title} ({item.count})"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d={item.icon} />
          </svg>
          {#if item.count > 0}<span class="rail-count">{item.count}</span>{/if}
        </button>
      {/each}
    </div>
  {:else}
  <div class="sections">
    <!-- The way out, when this repository sits inside another one. Read from
         git rather than remembered, so it is there however the user arrived. -->
    {#if parent}
      <button
        class="row parent"
        class:unreachable={parentReason !== undefined}
        disabled={parentReason !== undefined}
        onclick={() => onOpenRepo(parent.path)}
        title={parentReason
          ? `${parent.name} — ${parentReason}`
          : `${parent.path} — open the ${parent.kind === 'superproject' ? 'superproject' : 'main working tree'}`}
      >
        <svg class="glyph" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
          <path d={ICON_UP} />
        </svg>
        <span class="label">{parent.name}</span>
        <span class="badge"
          >{parent.kind === 'superproject' ? 'superproject' : 'main tree'}</span
        >
      </button>
    {/if}

    {@render section('local')}
    {#if openSections.local}
      {@render folderTree(tree(locals), 'local', 1)}
    {/if}

    {@render section('remote')}
    {#if openSections.remote}
      {#each remotes as remote (remote.name)}
        {@const key = `remote/${remote.name}`}
        <button class="row folder" style="padding-left: 20px" onclick={() => toggleGroup(key)}>
          <span class="caret" class:open={groupOpen(key)}>›</span>
          <svg class="glyph" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d={ICON_CLOUD} />
          </svg>
          <span class="label">{remote.name}</span>
          <span class="count">{visible(remote.entries).length}</span>
        </button>
        {#if groupOpen(key)}
          {@render folderTree(tree(remote.entries), key, 2)}
        {/if}
      {/each}
    {/if}

    {@render section('worktrees')}
    {#if openSections.worktrees}
      {#each visible(worktrees) as wt (wt.name)}
        {@render leafRow(wt.name, wt, 1)}
      {/each}
    {/if}

    {@render section('tags')}
    {#if openSections.tags}
      {@render folderTree(tree(tags), 'tags', 1)}
    {/if}

    {@render section('submodules')}
    {#if openSections.submodules}
      {@render folderTree(tree(submodules), 'submodules', 1)}
    {/if}
  </div>
  {/if}
</aside>

<style>
  #repo-sidebar {
    width: 240px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-right: 1px solid var(--border);
    overflow: hidden;
    transition: width var(--transition);
  }

  /* Minimised: wide enough for a glyph and its count, nothing more. */
  #repo-sidebar.collapsed {
    width: 40px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px;
    flex-shrink: 0;
  }

  .collapse-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 26px;
    flex-shrink: 0;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    font-size: 15px;
    line-height: 1;
    cursor: pointer;
  }

  .collapse-btn:hover {
    background: var(--bg-hover);
    color: var(--text-bright);
  }

  .chevron {
    display: inline-block;
    transition: transform var(--transition);
  }

  .chevron.flipped {
    transform: rotate(180deg);
  }

  /* ── Minimised rail ─────────────────────────────────────────────────────── */
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding-top: 4px;
    border-top: 1px solid var(--border);
  }

  .rail-btn {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    background: none;
    border: none;
    border-radius: var(--radius-md);
    color: var(--text-muted);
    cursor: pointer;
  }

  .rail-btn:hover {
    background: var(--bg-hover);
    color: var(--text-bright);
  }

  .rail-btn svg {
    width: 15px;
    height: 15px;
  }

  /* The count rides the glyph's corner — the rail has no room for a column. */
  .rail-count {
    position: absolute;
    right: 1px;
    bottom: 1px;
    font-size: 8px;
    line-height: 1;
    padding: 1px 2px;
    border-radius: var(--radius-xs);
    background: var(--bg-elev);
    color: var(--text-dim);
  }

  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    height: 26px;
    background: var(--bg-hover);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .filter svg {
    width: 12px;
    height: 12px;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .filter input {
    flex: 1;
    min-width: 0;
    background: none;
    border: none;
    outline: none;
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
  }

  .filter input::placeholder {
    color: var(--text-dim);
  }

  .sections {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 8px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 24px;
    padding: 0 8px;
    background: none;
    border: none;
    color: var(--text-2);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .row:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  /* The way out of a submodule or a linked worktree. It sits above the
     sections and reads as a destination, not as one more ref. */
  .row.parent {
    height: 26px;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    margin-bottom: 2px;
  }
  .row.parent .label {
    font-weight: 600;
  }
  .row.parent .badge {
    text-transform: uppercase;
    font-size: 9px;
    letter-spacing: 0.06em;
  }

  .row.section {
    height: 26px;
    color: var(--text-dim);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    border-top: 1px solid var(--border);
    margin-top: 4px;
  }

  .row.folder {
    color: var(--text-muted);
  }

  .row.head {
    background: var(--accent-bg);
    color: var(--accent);
    font-weight: 600;
  }

  /* A collapsed folder must not swallow the checked-out branch: the folders on
     the way to it are tinted, without taking the accent background that marks
     the branch row itself. */
  .row.folder.holds-head {
    color: var(--accent);
  }
  .row.folder.holds-head .glyph {
    color: var(--accent);
  }

  .row.head:hover:not(:disabled) {
    background: var(--accent-bg-strong);
  }

  /* A ref whose commit has not loaded has no row to reveal — say so by greying it
     out rather than offering a click that cannot do anything. */
  .row.unreachable {
    color: var(--text-faint);
    cursor: default;
  }
  /* The working tree being viewed is marked, not faded: it is where the user
     already is, and inert only because there is nowhere left to go. */
  .row.head.unreachable {
    color: var(--accent);
  }

  .caret {
    display: inline-block;
    width: 10px;
    flex-shrink: 0;
    color: var(--text-dim);
    transition: transform var(--transition);
  }

  .caret.open {
    transform: rotate(90deg);
  }

  .glyph {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    color: var(--text-dim);
  }

  .row.head .glyph {
    color: var(--accent);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge,
  .count,
  .sha {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--text-dim);
  }

  /* Says outright what the accent and the tick only imply. A pill rather than
     more dim text, so it reads as a status and not as one more count. */
  .mark {
    flex-shrink: 0;
    padding: 1px 5px;
    border: 1px solid var(--accent-border-strong);
    border-radius: var(--radius-xs);
    background: var(--accent-bg-strong);
    color: var(--accent-bright);
    font-size: 9px;
    font-weight: 600;
    line-height: 1.4;
    white-space: nowrap;
  }

  .sha {
    font-family: var(--font-mono);
    color: var(--text-dimmer);
  }

  .row.head .badge {
    color: var(--accent);
  }
</style>
