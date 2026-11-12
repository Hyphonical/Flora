<script lang="ts">
  import {
    ChevronDown,
    ChevronUp,
    Terminal,
    Globe,
    Share2,
    CheckCircle2,
    X,
    Pause,
    Play,
    ArrowDownToLine,
    ArrowUpFromLine
  } from '@lucide/svelte';
  import { store, formatBytes, type TransferItem } from '../lib/store.svelte';

  const activeCount = $derived(store.activeTransfers.length);
  const totalSpeed = $derived(store.aggregateSpeedMbps);
  const hasCompleted = $derived(store.transfers.some((t) => t.phase === 'complete'));

  function sourceLabel(item: TransferItem): string {
    if (item.source === 'cli') return 'CLI';
    if (item.source === 'share') return 'Share';
    return 'Web';
  }
</script>

{#if store.dockVisible && store.transfers.length > 0}
  <section class="upload-dock" aria-label="Transfers">
    <div class="upload-dock-header">
      <button
        class="dock-toggle"
        onclick={() => (store.dockExpanded = !store.dockExpanded)}
      >
        <span
          class="live-dot"
          class:paused={!store.sseConnected || activeCount === 0}
        ></span>
        <span>
          {#if activeCount > 0}
            Transfers · {activeCount} active
          {:else}
            Transfers · Complete
          {/if}
        </span>
      </button>

      <div class="dock-controls">
        {#if activeCount > 0 && store.sseConnected}
          <span class="mono speed-readout">{totalSpeed.toFixed(1)} MB/s</span>
        {/if}

        <button
          class="dock-icon-btn"
          title={store.ssePaused ? 'Reconnect live stream' : 'Pause live stream'}
          onclick={() => store.toggleSseStream()}
        >
          {#if store.ssePaused}
            <Play size={13} />
          {:else}
            <Pause size={13} />
          {/if}
        </button>

        <button
          class="dock-icon-btn"
          title={store.dockExpanded ? 'Minimize' : 'Expand'}
          onclick={() => (store.dockExpanded = !store.dockExpanded)}
        >
          {#if store.dockExpanded}
            <ChevronDown size={15} />
          {:else}
            <ChevronUp size={15} />
          {/if}
        </button>

        <button
          class="dock-icon-btn"
          title="Hide transfers"
          onclick={() => (store.dockVisible = false)}
        >
          <X size={14} />
        </button>
      </div>
    </div>

    {#if store.dockExpanded}
      <div class="upload-dock-body">
        {#each store.transfers as item (item.id)}
          <div class="upload-job" class:completed={item.phase === 'complete'}>
            <div class="upload-job-top">
              <div class="job-name-wrap">
                <span
                  class="source-tag"
                  title={item.source === 'cli'
                    ? 'CLI transfer'
                    : item.source === 'share'
                      ? 'Share transfer'
                      : 'Browser transfer'}
                >
                  {#if item.source === 'cli'}
                    <Terminal size={12} />
                  {:else if item.source === 'share'}
                    <Share2 size={12} />
                  {:else}
                    <Globe size={12} />
                  {/if}
                </span>
                <span class="direction-tag" title={item.direction === 'upload' ? 'Upload' : 'Download'}>
                  {#if item.direction === 'upload'}
                    <ArrowUpFromLine size={12} />
                  {:else}
                    <ArrowDownToLine size={12} />
                  {/if}
                </span>
                <span class="source-label mono">{sourceLabel(item)}</span>
                <span class="job-name">{item.name}</span>
              </div>

              <div class="job-right">
                <span class="mono job-status" class:accent={item.phase === 'transferring'}>
                  {#if item.phase === 'transferring'}
                    {#if item.chunks_total}
                      {item.chunks_done ?? 0}/{item.chunks_total}
                    {:else}
                      {formatBytes(item.transferred_bytes)}
                    {/if}
                  {:else if item.phase === 'verifying'}
                    Verifying…
                  {:else if item.phase === 'storing'}
                    Storing…
                  {:else}
                    <CheckCircle2 size={13} class="done-icon" />
                    Done
                  {/if}
                </span>
                <button
                  class="remove-job-btn"
                  title={item.phase === 'complete'
                    ? 'Dismiss'
                    : item.direction === 'download'
                      ? 'Dismiss'
                      : 'Cancel upload'}
                  onclick={() => store.cancelOrDismissTransfer(item.id)}
                >
                  <X size={12} />
                </button>
              </div>
            </div>

            <div
              class="chunk-strip"
              style:grid-template-columns="repeat({item.chunks.length}, 1fr)"
            >
              {#each item.chunks as chunk, idx (idx)}
                <div class="chunk-cell {chunk}"></div>
              {/each}
            </div>

            <div class="upload-job-foot">
              <span class="mono dest-path">/{item.folder ? `${item.folder}/` : ''}</span>
              <span class="mono">
                {#if item.phase === 'transferring'}
                  {formatBytes(item.transferred_bytes)} / {formatBytes(item.total_bytes ?? 0)} · {(
                    item.speed_bps / 1_000_000
                  ).toFixed(1)} MB/s
                {:else}
                  {formatBytes(item.total_bytes ?? item.transferred_bytes)}
                {/if}
              </span>
            </div>
          </div>
        {/each}

        {#if hasCompleted}
          <div class="dock-footer">
            <button class="clear-done-btn" onclick={() => store.clearCompletedTransfers()}>
              Clear completed
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </section>
{/if}

<style>
  .upload-dock {
    position: fixed;
    right: 20px;
    bottom: 16px;
    width: 430px;
    max-width: calc(100vw - 28px);
    background: var(--surface-1);
    border: 1px solid var(--border-hover);
    border-radius: var(--r-md);
    box-shadow: 0 18px 42px rgba(0, 0, 0, 0.6);
    z-index: 35;
    overflow: hidden;
  }

  .upload-dock-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 11px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    gap: 8px;
  }

  .dock-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--head);
    text-align: left;
    flex: 1;
    min-width: 0;
  }

  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 3px rgba(175, 185, 122, 0.2);
    flex-shrink: 0;
  }

  .live-dot.paused {
    background: var(--dim);
    box-shadow: none;
  }

  .dock-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .speed-readout {
    font-size: 11px;
    color: var(--accent);
    margin-right: 4px;
  }

  .dock-icon-btn {
    display: inline-flex;
    align-items: center;
    gap: 1px;
    padding: 4px 5px;
    border-radius: 5px;
    color: var(--dim);
    transition: background 100ms, color 100ms;
  }

  .dock-icon-btn:hover {
    background: var(--surface-3);
    color: var(--head);
  }

  .upload-dock-body {
    padding: 10px 13px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 300px;
    overflow-y: auto;
  }

  .upload-job {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-bottom: 9px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .upload-job:last-child {
    padding-bottom: 0;
    border-bottom: none;
  }

  .upload-job.completed {
    opacity: 0.78;
  }

  .upload-job-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }

  .job-name-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .source-tag,
  .direction-tag {
    color: var(--dim);
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
  }

  .direction-tag {
    color: var(--accent);
  }

  .source-label {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 5px;
    flex-shrink: 0;
  }

  .job-name {
    color: var(--head);
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .job-right {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .job-status {
    font-size: 11px;
    color: var(--dim);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .job-status.accent {
    color: var(--accent);
  }

  :global(.done-icon) {
    color: var(--accent);
  }

  .remove-job-btn {
    color: var(--muted);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
  }

  .remove-job-btn:hover {
    color: var(--head);
    background: var(--surface-3);
  }

  .chunk-strip {
    display: grid;
    gap: 2.5px;
    height: 6px;
  }

  .chunk-cell {
    border-radius: 2px;
    background: #2a2d2e;
    transition: background 180ms ease;
  }

  .chunk-cell.done {
    background: var(--accent);
  }

  .chunk-cell.inflight {
    background: var(--warn);
  }

  .upload-job-foot {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--dim);
    gap: 8px;
  }

  .dest-path {
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dock-footer {
    display: flex;
    justify-content: flex-end;
    padding-top: 2px;
  }

  .clear-done-btn {
    font-size: 11px;
    color: var(--dim);
  }

  .clear-done-btn:hover {
    color: var(--head);
  }

  @media (max-width: 640px) {
    .upload-dock {
      left: 10px;
      right: 10px;
      bottom: 10px;
      width: auto;
      max-width: none;
    }
  }
</style>
