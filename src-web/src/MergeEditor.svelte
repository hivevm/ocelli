<script lang="ts">
  // The merge editor (ADR-0031, G-15): a conflicted file as the working directory holds it, its
  // blocks set apart by their marker lines, the text editable everywhere with undo. Saving takes
  // the result as the file's resolution; leaving with a result not saved asks first (Q-7).
  import { EditorState, RangeSetBuilder, StateField } from '@codemirror/state';
  import { Decoration, EditorView, keymap, lineNumbers, type DecorationSet } from '@codemirror/view';
  import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
  import { blockLines } from './mergeEditor.ts';
  import type { MergeFile } from './types.ts';

  interface Props {
    path: string;
    file: MergeFile;
    /** Why the last save was refused, shown until the next. */
    refused: string | null;
    onSave: (result: string) => void;
    onLeave: (result: string) => void;
  }
  let { path, file, refused, onSave, onLeave }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | null = null;

  /** The blocks' lines, each set apart by what it is, moved along with the edits around them. */
  function blockField(file: MergeFile) {
    return StateField.define<DecorationSet>({
      create(state) {
        const builder = new RangeSetBuilder<Decoration>();
        for (const [line, kind] of blockLines(file.blocks)) {
          if (line > state.doc.lines) break;
          builder.add(state.doc.line(line).from, state.doc.line(line).from,
            Decoration.line({ class: `cm-merge-${kind}` }));
        }
        return builder.finish();
      },
      update: (lines, tr) => lines.map(tr.changes),
      provide: (field) => EditorView.decorations.from(field),
    });
  }

  function result(): string {
    return view?.state.doc.toString() ?? '';
  }

  $effect(() => {
    const editor = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: file.text,
        extensions: [
          lineNumbers(),
          history(),
          blockField(file),
          keymap.of([
            { key: 'Mod-s', preventDefault: true, run: () => { onSave(result()); return true; } },
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
    <button class="btn primary" title="Save as the file's resolution (Ctrl+S)"
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
