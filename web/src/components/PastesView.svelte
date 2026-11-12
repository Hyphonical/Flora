<script lang="ts">
  import { untrack } from 'svelte';
  import { Plus, Share2, Save, Trash2, ArrowLeft } from '@lucide/svelte';
  import { store, formatBytes } from '../lib/store.svelte';
  import CodeEditor from '../lib/components/CodeEditor.svelte';

  let draftPath = $state('');
  let draftContent = $state('');
  let mobileShowEditor = $state(false);
  let loading = $state(false);

  const pastes = $derived(store.pasteFiles);
  const active = $derived(store.activePaste);
  const lineCount = $derived(Math.max(1, (draftContent || '').split('\n').length));

  let loadedPastePath = '';

  $effect(() => {
    const path = active?.path;
    if (!path || path === loadedPastePath) return;
    loadedPastePath = path;
    draftPath = path;
    loading = true;
    let cancelled = false;
    void untrack(() => store.ensureTextLoaded(path)).then((content) => {
      if (cancelled) return;
      draftContent = content;
      loading = false;
    });
    return () => {
      cancelled = true;
    };
  });

  function selectPaste(path: string) {
    store.activePastePath = path;
    mobileShowEditor = true;
  }
</script>

<div class="pastes-view">
  <div class="page-header">
    <div>
      <h1 class="editorial-title">Pastes</h1>
      <div class="page-meta-line">
        <span>{pastes.length} text {pastes.length === 1 ? 'file' : 'files'}</span>
      </div>
    </div>

    <div class="header-controls">
      <button
        class="btn btn-primary"
        onclick={() => {
          store.createNewPaste();
          mobileShowEditor = true;
        }}
      >
        <Plus size={14} />
        <span>New paste</span>
      </button>
    </div>
  </div>

  <div class="paste-grid" class:mobile-editing={mobileShowEditor}>
    <div class="paste-list">
      {#each pastes as p (p.path)}
        <button
          class="paste-item"
          class:active={active?.path === p.path}
          onclick={() => selectPaste(p.path)}
        >
          <div class="paste-item-top">
            <span class="paste-name">{p.name}</span>
            <span class="mono paste-size">{formatBytes(p.bytes)}</span>
          </div>
          <div class="paste-item-sub">
            <span class="mono">/{p.path}</span>
            {#if p.sharedToken}
              <span class="badge shared">shared</span>
            {/if}
          </div>
        </button>
      {/each}
    </div>

    {#if active}
      <div class="paste-editor-card">
        <div class="paste-toolbar">
          <div class="toolbar-left">
            <button
              class="btn btn-sm mobile-back"
              onclick={() => (mobileShowEditor = false)}
            >
              <ArrowLeft size={13} />
              <span>List</span>
            </button>
            <span class="mono slash">/</span>
            <input
              class="input mono path-input"
              value={draftPath}
              oninput={(e) => (draftPath = e.currentTarget.value)}
              aria-label="Paste file path"
            />
          </div>

          <div class="toolbar-actions">
            <span class="mono line-stat">{lineCount} ln</span>
            <button
              class="btn btn-sm"
              onclick={() => store.toggleShareLink(active)}
            >
              <Share2 size={13} />
              <span>{active.sharedToken ? 'Shared' : 'Share'}</span>
            </button>
            <button
              class="btn btn-danger btn-sm"
              onclick={() => {
                void store.moveToTrash(active.path);
                mobileShowEditor = false;
              }}
              title="Move to trash"
            >
              <Trash2 size={13} />
            </button>
            <button
              class="btn btn-primary btn-sm"
              onclick={() => store.saveFileContent(active.path, draftPath, draftContent)}
            >
              <Save size={13} />
              <span>Save</span>
            </button>
          </div>
        </div>

        {#if loading}
          <div class="paste-loading mono">Loading paste…</div>
        {:else}
          <CodeEditor bind:code={draftContent} filename={active.name} minHeight={410} />
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .paste-grid {
    display: grid;
    grid-template-columns: 270px minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }

  .paste-list {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .paste-item {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-subtle);
    text-align: left;
    transition: background 100ms;
  }

  .paste-item:last-child {
    border-bottom: none;
  }

  .paste-item:hover {
    background: var(--surface-2);
  }

  .paste-item.active {
    background: rgba(175, 185, 122, 0.1);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .paste-item-top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }

  .paste-name {
    font-weight: 500;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .paste-item.active .paste-name {
    color: var(--head);
  }

  .paste-size {
    font-size: 11px;
    color: var(--dim);
    flex-shrink: 0;
  }

  .paste-item-sub {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--dim);
    margin-top: 4px;
  }

  .paste-item-sub span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .paste-editor-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .paste-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px 12px;
    background: var(--surface-0);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 180px;
  }

  .slash {
    color: var(--dim);
  }

  .path-input {
    flex: 1;
    max-width: 320px;
  }

  .toolbar-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .line-stat {
    font-size: 11px;
    color: var(--dim);
  }

  .mobile-back {
    display: none;
  }

  .paste-loading {
    padding: 20px 14px;
    color: var(--dim);
    font-size: 12px;
  }

  @media (max-width: 860px) {
    .paste-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .paste-grid.mobile-editing .paste-list {
      display: none;
    }

    .paste-grid:not(.mobile-editing) .paste-editor-card {
      display: none;
    }

    .mobile-back {
      display: inline-flex;
    }
  }
</style>
