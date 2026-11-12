<script lang="ts">
  import {
    Folder,
    FileText,
    FileImage,
    FileAudio,
    FileArchive,
    FileSpreadsheet
  } from '@lucide/svelte';
  import type { FileKind } from '../lib/store.svelte';

  let { kind, size = 15 }: { kind: FileKind; size?: number } = $props();
</script>

<div class="glyph {kind}">
  {#if kind === 'folder'}
    <Folder {size} strokeWidth={1.75} />
  {:else if kind === 'image'}
    <FileImage {size} strokeWidth={1.75} />
  {:else if kind === 'audio'}
    <FileAudio {size} strokeWidth={1.75} />
  {:else if kind === 'archive'}
    <FileArchive {size} strokeWidth={1.75} />
  {:else if kind === 'pdf'}
    <FileSpreadsheet {size} strokeWidth={1.75} />
  {:else}
    <FileText {size} strokeWidth={1.75} />
  {/if}
</div>

<style>
  .glyph {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--dim);
  }

  .glyph.folder {
    color: var(--accent);
    background: rgba(175, 185, 122, 0.09);
    border-color: rgba(175, 185, 122, 0.3);
  }

  .glyph.text {
    color: var(--warn);
    background: rgba(194, 168, 108, 0.09);
    border-color: rgba(194, 168, 108, 0.28);
  }

  .glyph.image {
    color: #9bb8b3;
    background: rgba(138, 161, 158, 0.1);
    border-color: rgba(138, 161, 158, 0.28);
  }
</style>
