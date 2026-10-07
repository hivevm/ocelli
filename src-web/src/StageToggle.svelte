<script lang="ts">
  // Whether a file of the uncommitted changes is staged, and the control that stages it whole or
  // takes it out of the index (G-4). A click here does not open the file's diff.
  import type { FileChange } from './types.ts';

  interface Props {
    file: FileChange;
    disabled: boolean;
    onStage: (file: FileChange) => void;
  }
  let { file, disabled, onStage }: Props = $props();

  const label = $derived(
    file.staged === 'yes' ? 'Unstage' : file.staged === 'partly' ? 'Stage the rest of' : 'Stage',
  );
</script>

<input
  type="checkbox"
  class="stage"
  checked={file.staged === 'yes'}
  indeterminate={file.staged === 'partly'}
  {disabled}
  title="{label} {file.path}"
  aria-label="{label} {file.path}"
  onclick={(e) => { e.stopPropagation(); e.preventDefault(); onStage(file); }}
  onkeydown={(e) => e.stopPropagation()}
/>

<style>
  .stage {
    flex-shrink: 0;
    margin: 0 6px 0 0;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .stage:disabled {
    cursor: default;
  }
</style>
