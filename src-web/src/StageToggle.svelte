<script lang="ts">
  // Whether a file of the uncommitted changes is staged, and the control that stages it whole or
  // takes it out of the index (G-12). A click here does not open the file's diff.
  import type { FileChange, Staging } from './types.ts';

  interface Props {
    file: FileChange;
    /** How much of the file's change is staged. */
    staged: Staging['staged'];
    disabled: boolean;
    onStage: (file: FileChange) => void;
  }
  let { file, staged, disabled, onStage }: Props = $props();

  const label = $derived(
    staged === 'yes' ? 'Unstage' : staged === 'partly' ? 'Stage the rest of' : 'Stage',
  );
</script>

<input
  type="checkbox"
  class="stage"
  checked={staged === 'yes'}
  indeterminate={staged === 'partly'}
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
