<script module lang="ts">
  // Row geometry, exported so the column header in App.svelte can mirror the
  // canvas layout exactly instead of guessing widths.
  const COL_WIDTH  = 20;
  const GRAPH_LEFT = 10;

  /** Width of the branch/tag column, which precedes the graph lanes. */
  export const REFS_WIDTH = 190;

  /** Lane-area width shown while no repository is loaded. */
  export const EMPTY_GRAPH_WIDTH = 180;

  /** Width of the graph lane area for a graph whose rightmost column is `maxCol`. */
  export function graphAreaWidth(maxCol: number): number {
    return GRAPH_LEFT + (maxCol + 1) * COL_WIDTH + GRAPH_LEFT;
  }
</script>

<script lang="ts">
  import type { GraphData, NodeJson, Edge } from './types.ts';
  import { FONTS, THEMES, seriesColor, type Theme } from './theme/palette.ts';

  // ── Props ──────────────────────────────────────────────────────────────────
  interface Props {
    graphData: GraphData | null;
    onSelectCommit?: (sha: string) => void;
    theme?: Theme;
    /** Rows of the selected commit's ancestry; when set, all other rows are dimmed. */
    highlightRows?: Set<number> | null;
    /**
     * The checked-out branch, so its chip can be marked as such. It comes from
     * the refs payload, which is read together with the graph (ADR-0026), so the
     * two cannot disagree about which branch that is.
     */
    headRef?: string | null;
  }
  let {
    graphData,
    onSelectCommit,
    theme = 'dark',
    highlightRows = null,
    headRef = null,
  }: Props = $props();

  // Dim levels while an ancestry highlight is active: edges and node glyphs
  // fade strongly so the highlighted lines pop; text stays readable.
  const DIM_GRAPH = 0.22;
  const DIM_TEXT  = 0.45;

  // ── Layout constants ───────────────────────────────────────────────────────
  const ROW_HEIGHT  = 26;
  const NODE_RADIUS = 5;
  const LINE_WIDTH  = 1.5;

  const W_SHA = 80;

  function branchColor(col: number): string {
    return seriesColor(col);
  }

  // ── Theme palettes (canvas can't read CSS vars cheaply per frame) ──────────
  interface Chip { bg: string; text: string; border: string; }
  interface Palette {
    bg: string; emptyTitle: string; emptySub: string;
    rowSel: string; accent: string; rowHover: string; rowSep: string;
    separator: string; nodeFill: string;
    shaSel: string; sha: string; msgSel: string; msg: string;
    chipTag: Chip; chipRemote: Chip; chipLocal: Chip; chipMore: Chip; chipHead: Chip;
  }
  // Most entries are the shared tokens; the few literals are graph-specific
  // shades with no CSS counterpart (the row tint behind a selected commit, the
  // dimmed SHA column, the "+n more" chip, and the HEAD chip's brighter text).
  const d = THEMES.dark;
  const l = THEMES.light;
  const PALETTES: Record<Theme, Palette> = {
    dark: {
      bg: d.bgApp, emptyTitle: d.textFaint, emptySub: d.border,
      rowSel: '#1e2a3a', accent: d.accent, rowHover: d.bgHover, rowSep: d.bgHover,
      separator: d.border, nodeFill: d.bgApp,
      shaSel: d.blue, sha: '#4a5070', msgSel: d.textBrightest, msg: d.text2,
      chipTag:    { bg: d.amberBg, text: d.amber,  border: d.amberBorder },
      chipRemote: { bg: d.blueBg,  text: d.blue,   border: d.blueBorder },
      chipLocal:  { bg: d.accentBg, text: d.accent, border: d.accentBorder },
      chipMore:   { bg: '#23262e', text: '#8a8f9e', border: '#343845' },
      chipHead:   { bg: d.accentBgStrong, text: '#7fe0aa', border: d.accent },
    },
    light: {
      bg: l.bgApp, emptyTitle: l.textFaint, emptySub: l.borderStrong,
      rowSel: l.bgSel, accent: l.accent, rowHover: l.bgHover, rowSep: '#eef0f3',
      separator: l.border, nodeFill: l.bgPanel,
      shaSel: l.blue, sha: l.textDimmer, msgSel: l.textBrightest, msg: l.text2,
      chipTag:    { bg: l.amberBg, text: l.amber,  border: l.amberBorder },
      chipRemote: { bg: l.blueBg,  text: l.blue,   border: l.blueBorder },
      chipLocal:  { bg: l.accentBg, text: l.accent, border: l.accentBorder },
      chipMore:   { bg: '#eef0f3', text: '#6b7180', border: '#d3d7de' },
      chipHead:   { bg: '#cdead6', text: '#1f7a4c', border: l.accent },
    },
  };
  const pal: Palette = $derived(PALETTES[theme]);

  // ── Local state ────────────────────────────────────────────────────────────
  let scrollY     = $state(0);
  let hoveredRow  = $state(-1);
  let selectedRow = $state(-1);

  // ── Canvas element + context ───────────────────────────────────────────────
  let canvas = $state<HTMLCanvasElement | null>(null);
  let ctx: CanvasRenderingContext2D | null = $derived(canvas?.getContext('2d') ?? null);

  // ── Layout helpers ─────────────────────────────────────────────────────────
  // Lane 0 starts after the branch/tag column, so the whole graph is offset by it.
  function nodeX(col: number): number { return REFS_WIDTH + GRAPH_LEFT + col * COL_WIDTH + COL_WIDTH / 2; }
  function nodeY(row: number): number { return row * ROW_HEIGHT + ROW_HEIGHT / 2; }

  // ── Row index, rebuilt only when the graph changes ─────────────────────────
  // Rendering must cost O(visible rows), not O(history) — see "Virtual rendering"
  // in docs/SPECIFICATION.md. Scanning every node and edge per frame would make it
  // O(n), so nodes are indexed by row and edges are bucketed by their first row.
  // Edges spanning more than a viewport's worth of rows are kept in a separate
  // list, so a bounded bucket window plus that short list covers every edge that
  // can intersect the viewport.
  const MAX_SHORT_SPAN = 256;

  const index = $derived.by(() => {
    const nodes = graphData?.nodes ?? [];
    const edges = graphData?.edges ?? [];
    const rowCount = nodes.length;

    const nodesByRow: (NodeJson | undefined)[] = new Array(rowCount);
    let maxCol = 0;
    for (const n of nodes) {
      if (n.row >= 0 && n.row < rowCount) nodesByRow[n.row] = n;
      if (n.col > maxCol) maxCol = n.col;
    }

    const shortByFirstRow: Edge[][] = Array.from({ length: rowCount }, () => []);
    const longEdges: Edge[] = [];
    for (const e of edges) {
      if (e.minRow < 0 || e.minRow >= rowCount || e.maxRow - e.minRow > MAX_SHORT_SPAN) {
        longEdges.push(e);
      } else {
        shortByFirstRow[e.minRow].push(e);
      }
    }

    return {
      nodesByRow,
      shortByFirstRow,
      longEdges,
      rowCount,
      width: rowCount === 0 ? EMPTY_GRAPH_WIDTH : graphAreaWidth(maxCol),
    };
  });

  function totalContentHeight(): number {
    return index.rowCount * ROW_HEIGHT;
  }

  // ── Canvas sizing ──────────────────────────────────────────────────────────
  function resizeCanvas(): void {
    if (!canvas) return;
    const container = canvas.parentElement!;
    canvas.width  = container.clientWidth;
    canvas.height = container.clientHeight;
    render();
  }

  $effect(() => {
    if (!canvas) return;
    const observer = new ResizeObserver(() => resizeCanvas());
    observer.observe(canvas.parentElement!);
    resizeCanvas();
    return () => observer.disconnect();
  });

  $effect(() => {
    void [graphData, scrollY, hoveredRow, selectedRow, theme, highlightRows, headRef];
    render();
  });

  $effect(() => {
    if (graphData) {
      scrollY     = 0;
      hoveredRow  = -1;
      selectedRow = -1;
    }
  });

  // ── Main render ────────────────────────────────────────────────────────────
  function render(): void {
    if (!canvas || !ctx) return;
    const W = canvas.width;
    const H = canvas.height;

    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = pal.bg;
    ctx.fillRect(0, 0, W, H);

    if (index.rowCount === 0) {
      ctx.fillStyle = pal.emptyTitle;
      ctx.font = `13px ${FONTS.ui}`;
      ctx.textAlign = 'center';
      ctx.fillText('Open a Git repository to get started', W / 2, H / 2 - 10);
      ctx.font = `11px ${FONTS.ui}`;
      ctx.fillStyle = pal.emptySub;
      ctx.fillText('Use "Open Repo" in the toolbar above', W / 2, H / 2 + 12);
      ctx.textAlign = 'left';
      return;
    }

    const maxScroll = Math.max(0, totalContentHeight() - H);
    scrollY = Math.min(Math.max(scrollY, 0), maxScroll);

    const visRowMin = Math.max(0, Math.floor(scrollY / ROW_HEIGHT));
    const visRowMax = Math.min(index.rowCount - 1, Math.ceil((scrollY + H) / ROW_HEIGHT));
    // Columns: [branch/tag][graph lanes][sha, message]
    const infoX = REFS_WIDTH + index.width;

    // Row backgrounds
    for (let r = visRowMin; r <= visRowMax; r++) {
      const y = r * ROW_HEIGHT - scrollY;
      if (r === selectedRow) {
        ctx.fillStyle = pal.rowSel;
        ctx.fillRect(0, y, W, ROW_HEIGHT);
        // left accent bar for selected
        ctx.fillStyle = pal.accent;
        ctx.fillRect(0, y, 2, ROW_HEIGHT);
      } else if (r === hoveredRow) {
        ctx.fillStyle = pal.rowHover;
        ctx.fillRect(0, y, W, ROW_HEIGHT);
      }
      // subtle separator between rows
      ctx.strokeStyle = pal.rowSep;
      ctx.lineWidth = 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y + ROW_HEIGHT - 0.5);
      ctx.lineTo(W, y + ROW_HEIGHT - 0.5);
      ctx.stroke();
    }

    // Column separators: refs/graph and graph/info
    ctx.strokeStyle = pal.separator;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(REFS_WIDTH, 0);
    ctx.lineTo(REFS_WIDTH, H);
    ctx.moveTo(infoX, 0);
    ctx.lineTo(infoX, H);
    ctx.stroke();

    // Edges — orthogonal L-shapes with a short curve at the corner.
    // An edge is drawn when its row span intersects the visible range, so edges
    // running in from off-screen rows are still drawn.
    ctx.lineWidth = LINE_WIDTH;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    const bucketFrom = Math.max(0, visRowMin - MAX_SHORT_SPAN);
    for (let r = bucketFrom; r <= visRowMax; r++) {
      for (const edge of index.shortByFirstRow[r]) {
        if (edge.maxRow < visRowMin) continue;
        drawEdge(edge);
      }
    }
    for (const edge of index.longEdges) {
      if (edge.maxRow < visRowMin || edge.minRow > visRowMax) continue;
      drawEdge(edge);
    }

    // Nodes — one row lookup per visible row, already in row order
    for (let r = visRowMin; r <= visRowMax; r++) {
      const node = index.nodesByRow[r];
      if (node) drawNode(node, infoX, W);
    }
  }

  // ── Edge rendering with short corner curves (GitKraken style) ────────────
  function drawEdge(edge: Edge): void {
    // An edge is on the ancestry path iff both endpoints are: every loaded
    // parent of an ancestry node is itself ancestry, so child→parent edges
    // inside the set stay bright and everything else fades.
    const dimmed =
      highlightRows !== null &&
      !(highlightRows.has(edge.sourceRow) && highlightRows.has(edge.targetRow));
    if (dimmed) ctx!.globalAlpha = DIM_GRAPH;

    const x1 = nodeX(edge.sourceCol);
    const y1 = nodeY(edge.sourceRow) - scrollY;
    const x2 = nodeX(edge.targetCol);
    const y2 = nodeY(edge.targetRow) - scrollY;

    const color = branchColor(edge.isBranch ? edge.targetCol : edge.sourceCol);
    ctx!.strokeStyle = color;
    ctx!.beginPath();
    ctx!.moveTo(x1, y1);

    if (x1 === x2) {
      // Straight vertical
      ctx!.lineTo(x2, y2);
    } else {
      const r = COL_WIDTH / 2;
      const dx = x2 > x1 ? 1 : -1; // direction: +1 right, -1 left
      const dy = y2 > y1 ? 1 : -1; // direction: +1 down,  -1 up

      // The shape follows the edge type, not the side the child is on
      // (ADR-0030): each vertical run then lies in a column the layout keeps
      // clear, so no edge can pass through a commit.
      if (edge.isBranch) {
        // BRANCH: horizontal first, then vertical up the child's lane.
        // Bend sits at the target column (x2), at the source row (y1).
        ctx!.lineTo(x2 - dx * r, y1);
        ctx!.quadraticCurveTo(x2, y1, x2, y1 + dy * r);
        ctx!.lineTo(x2, y2);
      } else {
        // MERGE: vertical first, up the parent's column, then horizontal.
        // Bend sits at the source column (x1), at the target row (y2).
        ctx!.lineTo(x1, y2 - dy * r);
        ctx!.quadraticCurveTo(x1, y2, x1 + dx * r, y2);
        ctx!.lineTo(x2, y2);
      }
    }
    ctx!.stroke();
    ctx!.globalAlpha = 1;
  }

  // ── Node rendering ─────────────────────────────────────────────────────────
  function drawNode(node: NodeJson, infoX: number, canvasW: number): void {
    const cx = nodeX(node.col);
    const cy = nodeY(node.row) - scrollY;
    const color = branchColor(node.col);
    const isSelected = node.row === selectedRow;
    const dimmed = highlightRows !== null && !highlightRows.has(node.row);
    if (dimmed) ctx!.globalAlpha = DIM_GRAPH;

    // A glyph's fill is the canvas background: it punches a hole in the edge
    // running behind the node, which is what keeps the line from crossing the
    // circle. Dimming that hole along with everything else would make the edge
    // show through the glyph, so the fill always goes down opaque and only the
    // outline fades. A selected node is never dimmed, so its coloured fill is
    // unaffected either way.
    const fillGlyph = (): void => {
      ctx!.globalAlpha = 1;
      ctx!.fill();
      if (dimmed) ctx!.globalAlpha = DIM_GRAPH;
    };

    // Node outer glow for selected
    if (isSelected) {
      ctx!.beginPath();
      ctx!.arc(cx, cy, NODE_RADIUS + 3, 0, Math.PI * 2);
      ctx!.fillStyle = color + '33';
      ctx!.fill();
    }

    // Node glyph — circle for commits, diamond for stashes, dashed for WIP
    if (node.kind === 'stash') {
      const r = NODE_RADIUS + 0.5;
      ctx!.beginPath();
      ctx!.moveTo(cx, cy - r);
      ctx!.lineTo(cx + r, cy);
      ctx!.lineTo(cx, cy + r);
      ctx!.lineTo(cx - r, cy);
      ctx!.closePath();
      ctx!.fillStyle = isSelected ? color : pal.nodeFill;
      fillGlyph();
      ctx!.strokeStyle = color;
      ctx!.lineWidth = isSelected ? 2 : 1.5;
      ctx!.stroke();
    } else if (node.kind === 'working') {
      ctx!.beginPath();
      ctx!.arc(cx, cy, NODE_RADIUS, 0, Math.PI * 2);
      ctx!.fillStyle = pal.nodeFill;
      fillGlyph();
      ctx!.setLineDash([2, 2]);
      ctx!.strokeStyle = color;
      ctx!.lineWidth = isSelected ? 2 : 1.5;
      ctx!.stroke();
      ctx!.setLineDash([]);
    } else {
      // Node fill
      ctx!.beginPath();
      ctx!.arc(cx, cy, NODE_RADIUS, 0, Math.PI * 2);
      ctx!.fillStyle = isSelected ? color : pal.nodeFill;
      fillGlyph();

      // Node border
      ctx!.beginPath();
      ctx!.arc(cx, cy, NODE_RADIUS, 0, Math.PI * 2);
      ctx!.strokeStyle = color;
      ctx!.lineWidth = isSelected ? 2 : 1.5;
      ctx!.stroke();

      // Merge commits (more than one parent) get an inner ring — the
      // "double circle" from the README roadmap.
      if (node.parents.length > 1) {
        ctx!.beginPath();
        ctx!.arc(cx, cy, NODE_RADIUS - 2.5, 0, Math.PI * 2);
        ctx!.strokeStyle = isSelected ? pal.nodeFill : color;
        ctx!.lineWidth = 1;
        ctx!.stroke();
      }
    }
    ctx!.lineWidth = LINE_WIDTH;
    if (dimmed) ctx!.globalAlpha = DIM_TEXT;

    const baseY  = node.row * ROW_HEIGHT - scrollY;
    const textY  = baseY + Math.round((ROW_HEIGHT + 11) / 2) - 1;

    // Branch/tag chips — their own column, left of the graph lanes.
    drawRefChips(node.refs, baseY);

    let x = infoX + 8;

    // SHA
    ctx!.font = `11px ${FONTS.mono}`;
    ctx!.fillStyle = isSelected ? pal.shaSel : pal.sha;
    ctx!.fillText(node.shortSha, x, textY);
    x += W_SHA;

    // Message — takes every pixel left of the canvas edge. Author and date live
    // in the commit detail sidebar, not in the row.
    const msgW = Math.max(60, canvasW - 8 - x);

    ctx!.font = isSelected
      ? `500 12px ${FONTS.ui}`
      : `12px ${FONTS.ui}`;
    ctx!.fillStyle = isSelected ? pal.msgSel : pal.msg;
    ctx!.fillText(truncateText(node.message, msgW), x, textY);
    ctx!.globalAlpha = 1;
  }

  // ── Ref chip rendering ─────────────────────────────────────────────────────
  const CHIP_H   = 15;
  const CHIP_PAD = 6;  // horizontal padding inside a chip, per side
  const CHIP_GAP = 4;  // space between two chips

  /** What a chip is, which decides both its colours and its label. */
  type ChipKind = 'head' | 'tag' | 'remote' | 'local' | 'more';

  interface PlannedChip { label: string; width: number; kind: ChipKind; }

  /// Classifies one ref name. The `HEAD` test runs first: a branch name may
  /// contain a slash, and `feat/x` must not be mistaken for a remote branch.
  function chipKind(ref: string): ChipKind {
    if (headRef !== null && ref === headRef) return 'head';
    if (ref.startsWith('🏷')) return 'tag';
    return ref.includes('/') ? 'remote' : 'local';
  }

  function chipPalette(kind: ChipKind): Chip {
    switch (kind) {
      case 'head':   return pal.chipHead;
      case 'tag':    return pal.chipTag;
      case 'remote': return pal.chipRemote;
      case 'more':   return pal.chipMore;
      default:       return pal.chipLocal;
    }
  }

  /// The text drawn in the chip. The checked-out branch carries a tick, so it
  /// says what it is even where the colour is hard to judge — next to another
  /// green chip, or for a reader who cannot tell the two greens apart.
  function chipLabel(ref: string, kind: ChipKind): string {
    return kind === 'head' ? `✓ ${ref}` : ref;
  }

  /// Widths of a chip's text and of the chip around it.
  function chipWidth(label: string): number {
    return ctx!.measureText(label).width + 2 * CHIP_PAD;
  }

  /// Decides which of a row's chips fit into the refs column.
  ///
  /// Chips are kept in order and the ones that do not fit collapse into a single
  /// `+N` badge, so a commit with many refs still shows that they exist. Two
  /// rules protect what matters most: the checked-out branch is moved to the
  /// front, because it must never be the chip that gets collapsed away, and the
  /// leading chip is shrunk before it is dropped — a row showing nothing but a
  /// count says which refs it holds no better than an empty row does.
  function planRefChips(refs: string[]): PlannedChip[] {
    const decorated = [...refs]
      .sort((a, b) => Number(chipKind(b) === 'head') - Number(chipKind(a) === 'head'))
      .map(ref => {
        const kind = chipKind(ref);
        return { kind, label: chipLabel(ref, kind) };
      });

    const avail = REFS_WIDTH - 16;
    const plan: PlannedChip[] = [];
    let used = 0;

    for (const chip of decorated) {
      const width = chipWidth(chip.label);
      const gap = plan.length === 0 ? 0 : CHIP_GAP;
      if (used + gap + width > avail) break;
      plan.push({ ...chip, width });
      used += gap + width;
    }

    let hidden = decorated.length - plan.length;
    if (hidden === 0) return plan;

    // Make room for the badge. Trailing chips go first; once only the leading
    // one is left it is truncated instead, so the row keeps a name on it.
    for (;;) {
      // Truncating the leading chip can leave nothing hidden after all, and a
      // "+0" badge would be a count of nothing.
      if (hidden === 0) return plan;

      const badge = chipWidth(`+${hidden}`);
      if (plan.length > 0 && used + CHIP_GAP + badge <= avail) {
        plan.push({ label: `+${hidden}`, width: badge, kind: 'more' });
        return plan;
      }

      if (plan.length > 1) {
        const dropped = plan.pop()!;
        used -= dropped.width + CHIP_GAP;
        hidden++;
        continue;
      }

      const room = avail - badge - CHIP_GAP - 2 * CHIP_PAD;
      if (room < 12) {
        // Not even a truncated chip and a badge fit: the count is all there is.
        const all = `+${decorated.length}`;
        return [{ label: all, width: chipWidth(all), kind: 'more' }];
      }

      const first = decorated[0];
      const label = truncateText(first.label, room);
      plan.length = 0;
      plan.push({ label, width: chipWidth(label), kind: first.kind });
      used = chipWidth(label);
      hidden = decorated.length - 1;
    }
  }

  /// Draws a row's branch/tag chips into the refs column, right-aligned so they
  /// end where the graph lanes begin.
  function drawRefChips(refs: string[], rowY: number): void {
    if (!refs || refs.length === 0) return;
    ctx!.font = `10px ${FONTS.ui}`;

    const plan = planRefChips(refs);
    if (plan.length === 0) return;

    const total = plan.reduce((sum, c) => sum + c.width, 0) + (plan.length - 1) * CHIP_GAP;
    const chipY = rowY + (ROW_HEIGHT - CHIP_H) / 2;
    const arc = 3;
    let x = REFS_WIDTH - 8 - total;

    for (const { label, width, kind } of plan) {
      const chip = chipPalette(kind);

      ctx!.fillStyle = chip.bg;
      roundRect(x, chipY, width, CHIP_H, arc);
      ctx!.fill();
      // The checked-out branch gets a heavier outline as well as its own
      // colours — one more difference that does not rely on hue.
      ctx!.strokeStyle = chip.border;
      ctx!.lineWidth = kind === 'head' ? 1.25 : 0.75;
      roundRect(x, chipY, width, CHIP_H, arc);
      ctx!.stroke();
      ctx!.lineWidth = LINE_WIDTH;
      ctx!.fillStyle = chip.text;
      ctx!.fillText(label, x + CHIP_PAD, chipY + CHIP_H - 3.5);
      x += width + CHIP_GAP;
    }
  }

  function roundRect(x: number, y: number, w: number, h: number, r: number): void {
    ctx!.beginPath();
    ctx!.moveTo(x + r, y);
    ctx!.lineTo(x + w - r, y);
    ctx!.arcTo(x + w, y,     x + w, y + r,     r);
    ctx!.lineTo(x + w, y + h - r);
    ctx!.arcTo(x + w, y + h, x + w - r, y + h, r);
    ctx!.lineTo(x + r, y + h);
    ctx!.arcTo(x,     y + h, x,     y + h - r, r);
    ctx!.lineTo(x,     y + r);
    ctx!.arcTo(x,     y,     x + r, y,         r);
    ctx!.closePath();
  }

  function truncateText(text: string, maxWidth: number): string {
    if (!text) return '';
    if (ctx!.measureText(text).width <= maxWidth) return text;
    let t = text;
    while (t.length > 1 && ctx!.measureText(t + '…').width > maxWidth) t = t.slice(0, -1);
    return t + '…';
  }

  // ── Mouse events ───────────────────────────────────────────────────────────
  function onMouseMove(e: MouseEvent): void {
    const rect = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const row = Math.floor((e.clientY - rect.top + scrollY) / ROW_HEIGHT);
    if (row !== hoveredRow) hoveredRow = row;
  }

  function onMouseLeave(): void {
    hoveredRow = -1;
  }

  function onClick(e: MouseEvent): void {
    const rect = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const row = Math.floor((e.clientY - rect.top + scrollY) / ROW_HEIGHT);
    if (row < 0 || row >= index.rowCount) return;
    selectedRow = row;
    const node = index.nodesByRow[row];
    if (node) onSelectCommit?.(node.sha);
  }

  function onWheel(e: WheelEvent): void {
    e.preventDefault();
    scrollY += e.deltaY;
  }

  /// Clears the row selection (Escape in App.svelte); the ancestry dim is
  /// driven by the selection in App.svelte and lifts with it.
  export function clearSelection(): void {
    selectedRow = -1;
  }

  /// Selects a row from outside (e.g. the detail sidebar's parent/child links)
  /// and scrolls it into view when it is off-screen. render() clamps scrollY.
  export function revealRow(row: number): void {
    if (row < 0 || row >= index.rowCount) return;
    selectedRow = row;
    const H = canvas?.height ?? 0;
    const top = row * ROW_HEIGHT;
    if (top < scrollY || top + ROW_HEIGHT > scrollY + H) {
      scrollY = top - H / 2 + ROW_HEIGHT / 2;
    }
  }
</script>

<canvas
  bind:this={canvas}
  onmousemove={onMouseMove}
  onmouseleave={onMouseLeave}
  onclick={onClick}
  onwheel={onWheel}
></canvas>

<style>
  canvas {
    display: block;
    cursor: default;
    width: 100%;
    height: 100%;
  }
</style>
