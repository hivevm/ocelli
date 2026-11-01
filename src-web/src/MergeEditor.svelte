<script lang="ts">
  // The merge editor (ADR-0031, G-15): a conflicted file as the working directory holds it, its
  // blocks set apart, ours above theirs, each with actions taking ours, theirs, or both, and its
  // base folded until opened; the text is editable everywhere with undo. Saving takes the result
  // as the file's resolution; leaving with a result not saved asks first (Q-7).
  import { Compartment, EditorState, type Range, type StateField } from '@codemirror/state';
  import { Decoration, EditorView, WidgetType, keymap, lineNumbers } from '@codemirror/view';
  import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
  import {
    blockLineKinds, blocksField, drawn, takeSide, toggledBase, undoableTakes, type BlockState,
  } from './mergeBlocks.ts';
  import type { Side } from './mergeEditor.ts';
  import type { MergeFile } from './types.ts';

  interface Props {
    path: string;
    file: MergeFile;
    /** Why the last save was refused, shown until the next. */
    refused: string | null;
    /** Whether a save runs: the text is not editable meanwhile, so no edit is lost when it ends. */
    saving: boolean;
    onSave: (result: string) => void;
    onLeave: (result: string) => void;
  }
  let { path, file, refused, saving, onSave, onLeave }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | null = null;

  /** The bar above a block: its actions, and the base's toggle where it has one. */
  class Actions extends WidgetType {
    readonly field: StateField<BlockState[]>;
    readonly index: number;
    readonly base: boolean;
    readonly open: boolean;

    constructor(field: StateField<BlockState[]>, index: number, base: boolean, open: boolean) {
      super();
      this.field = field;
      this.index = index;
      this.base = base;
      this.open = open;
    }

    eq(other: Actions): boolean {
      return other.index === this.index && other.base === this.base && other.open === this.open;
    }

    toDOM(view: EditorView): HTMLElement {
      const bar = document.createElement('div');
      bar.className = 'cm-merge-actions';
      const button = (label: string, title: string, run: () => void) => {
        const b = document.createElement('button');
        b.textContent = label;
        b.title = title;
        b.onmousedown = (e) => e.preventDefault();
        b.onclick = () => { if (!view.state.readOnly) run(); };
        bar.append(b);
      };
      const take = (side: Side) => () => {
        const spec = takeSide(view.state, this.field, this.index, side);
        if (spec) view.dispatch(spec);
      };
      button('Take ours', 'Replace the block by ours', take('ours'));
      button('Take theirs', 'Replace the block by theirs', take('theirs'));
      button('Take both', 'Replace the block by ours, then theirs', take('both'));
      if (this.base) {
        button(this.open ? 'Fold base' : 'Show base', 'The text both sides changed from',
          () => view.dispatch({ effects: toggledBase.of(this.index) }));
      }
      return bar;
    }
  }

  /** A base no line of the file holds, beneath its block. */
  class Base extends WidgetType {
    readonly text: string;

    constructor(text: string) {
      super();
      this.text = text;
    }

    eq(other: Base): boolean {
      return other.text === this.text;
    }

    toDOM(): HTMLElement {
      const base = document.createElement('pre');
      base.className = 'cm-merge-base cm-merge-base-merged';
      base.textContent = this.text.replace(/\r\n/g, '\n').replace(/\n$/, '');
      return base;
    }
  }

  /** The blocks drawn as they are now: each line set apart by what it is, the bar above each
      block, and its base folded, or open in place or beneath it. */
  function blockDecorations(field: StateField<BlockState[]>) {
    return EditorView.decorations.compute(['doc', field], (state) => {
      const blocks = state.field(field);
      const found: Range<Decoration>[] = [];
      for (const [line, kind] of blockLineKinds(state, blocks)) {
        found.push(Decoration.line({ class: `cm-merge-${kind}` }).range(state.doc.line(line).from));
      }
      for (const d of drawn(state, blocks)) {
        if (d.kind === 'bar') {
          found.push(Decoration.widget({
            widget: new Actions(field, d.index, d.base, d.open), block: true, side: -1,
          }).range(d.at));
        } else if (d.kind === 'fold') {
          found.push(Decoration.replace({ block: true }).range(d.from, d.to));
        } else {
          found.push(Decoration.widget({ widget: new Base(d.text), block: true, side: 1 }).range(d.at));
        }
      }
      return Decoration.set(found, true);
    });
  }

  const locked = new Compartment();

  function result(): string {
    return view?.state.doc.toString() ?? '';
  }

  /** The result as the editor holds it now. */
  export function current(): string {
    return result();
  }

  $effect(() => {
    view?.dispatch({ effects: locked.reconfigure(EditorState.readOnly.of(saving)) });
  });

  $effect(() => {
    const blocks = blocksField(file.blocks);
    const editor = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: file.text,
        extensions: [
          lineNumbers(),
          history(),
          locked.of(EditorState.readOnly.of(false)),
          blocks,
          undoableTakes(blocks),
          blockDecorations(blocks),
          keymap.of([
            { key: 'Mod-s', preventDefault: true, run: () => { if (!saving) onSave(result()); return true; } },
            { key: 'Escape', run: () => { onLeave(result()); return true; } },
            ...defaultKeymap,
            ...historyKeymap,
          ]),
        ],
      }),
    });
    view = editor;
    editor.focus();
    return () => {
      editor.destroy();
      view = null;
    };
  });
</script>

<div class="merge-panel" role="region" aria-label="Merge editor">
  <div class="merge-header">
    <span class="merge-path" title={path}>{path}</span>
    {#if refused}<span class="refused">{refused}</span>{/if}
    <button class="btn primary" title="Save as the file's resolution (Ctrl+S)" disabled={saving}
            onclick={() => onSave(result())}>Save</button>
    <button class="btn" title="Close (Esc)" onclick={() => onLeave(result())}>Close</button>
  </div>
  <div class="merge-text" bind:this={host}></div>
</div>

<style>
  .merge-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .merge-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .merge-path {
    font-family: var(--font-mono);
    color: var(--text-bright);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .refused {
    color: var(--error);
    font-size: 11px;
  }

  .merge-text {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .merge-text :global(.cm-editor) {
    height: 100%;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text);
    background: var(--bg-panel);
  }

  .merge-text :global(.cm-scroller) {
    overflow: auto;
  }

  .merge-text :global(.cm-gutters) {
    background: var(--diff-gutter-bg);
    color: var(--text-muted);
    border-right: 1px solid var(--border);
  }

  .merge-text :global(.cm-content) {
    caret-color: var(--text-bright);
  }

  .merge-text :global(.cm-merge-marker) {
    background: var(--diff-hunk-bg);
    color: var(--diff-hunk-text);
  }

  .merge-text :global(.cm-merge-ours) {
    background: var(--diff-add-bg);
  }

  .merge-text :global(.cm-merge-theirs) {
    background: var(--blue-bg);
  }

  .merge-text :global(.cm-merge-base) {
    background: var(--amber-bg);
  }

  .merge-text :global(.cm-merge-base-merged) {
    margin: 0;
    padding: 0 6px;
    font: inherit;
    color: var(--text-muted);
    background: var(--amber-bg);
    border-left: 2px solid var(--amber-border);
    white-space: pre-wrap;
  }

  .merge-text :global(.cm-merge-actions) {
    display: flex;
    gap: 4px;
    padding: 2px 6px;
    background: var(--diff-hunk-bg);
    border-top: 1px solid var(--border);
  }

  .merge-text :global(.cm-merge-actions button) {
    font: inherit;
    font-size: 11px;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .merge-text :global(.cm-merge-actions button:hover) {
    color: var(--accent);
    border-color: var(--accent-border);
  }

  .btn {
    font: inherit;
    font-size: 11px;
    padding: 2px 8px;
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
</style>
