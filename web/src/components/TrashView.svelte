<script lang="ts">
  import { RotateCcw, Trash2, X } from '@lucide/svelte';
  import { store, formatBytes, formatTimestamp, formatExpiresIn } from '../lib/store.svelte';
  import FileIcon from './FileIcon.svelte';

  const items = $derived(store.ownerTrash);
  const totalBytes = $derived(items.reduce((sum, item) => sum + item.size, 0));
  const user = $derived(store.currentUser);
</script>

<div>
  <div class="page-header">
    <div>
      <h1 class="editorial-title">Trash</h1>
      <div class="page-meta-line">
        <span>{items.length} {items.length === 1 ? 'item' : 'items'}</span>
        <span>·</span>
        <span class="mono">{formatBytes(totalBytes)}</span>
      </div>
    </div>

    <div class="header-controls">
      <label class="retention-pill">
        <span>Retention</span>
        <select
          value={user.retention}
          onchange={(e) => store.updateRetention(e.currentTarget.value)}
        >
          <option value="2d">2d</option>
          <option value="7d">7d</option>
          <option value="14d">14d</option>
          <option value="30d">30d</option>
          <option value="never">never</option>
        </select>
      </label>

      <button
        class="btn btn-danger"
        disabled={items.length === 0}
        onclick={() => store.emptyTrash()}
      >
        <Trash2 size={14} />
        <span>Empty trash</span>
      </button>
    </div>
  </div>

  {#if items.length === 0}
    <div class="empty-card">Trash is empty.</div>
  {:else}
    <div class="ledger">
      <div class="ledger-head trash-cols">
        <span>Original path</span>
        <span style="text-align:right;">Size</span>
        <span class="col-deleted">Deleted</span>
        <span class="col-expires">Expires</span>
        <span style="text-align:right;">Actions</span>
      </div>

      {#each items as item (item.id)}
        <div class="ledger-row trash-cols">
          <div class="path-cell">
            <FileIcon kind={item.directory ? 'folder' : 'archive'} />
            <span class="mono path-text">/{item.path}</span>
          </div>
          <div class="mono size-cell">{formatBytes(item.size)}</div>
          <div class="dim-cell col-deleted">{formatTimestamp(item.deleted)}</div>
          <div class="mono exp-cell col-expires">{formatExpiresIn(item.expires_at)}</div>
          <div class="act-cell">
            <button
              class="btn btn-sm"
              onclick={() => store.restoreFromTrash(item.id)}
            >
              <RotateCcw size={12} />
              <span>Restore</span>
            </button>
            <button
              class="btn btn-danger btn-sm"
              title="Delete permanently"
              onclick={() => store.deleteTrashItemPermanently(item.id)}
            >
              <X size={12} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .retention-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 5px 10px;
  }

  .retention-pill select {
    background: transparent;
    border: none;
    outline: none;
    color: var(--head);
    font-family: var(--font-mono);
    font-size: 12px;
    cursor: pointer;
  }

  .retention-pill select option {
    background: var(--surface-2);
  }

  .empty-card {
    padding: 40px 20px;
    text-align: center;
    color: var(--dim);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
  }

  .trash-cols {
    display: grid;
    grid-template-columns: minmax(200px, 1.4fr) 92px 130px 92px 150px;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
  }

  .path-cell {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .path-text {
    color: var(--fg);
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size-cell {
    text-align: right;
    color: var(--dim);
    font-size: 12px;
  }

  .dim-cell {
    color: var(--dim);
    font-size: 12px;
  }

  .exp-cell {
    color: var(--warn);
    font-size: 11.5px;
  }

  .act-cell {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  @media (max-width: 820px) {
    .trash-cols {
      grid-template-columns: minmax(150px, 1fr) 84px 98px;
    }

    .col-deleted,
    .col-expires {
      display: none;
    }
  }
</style>
