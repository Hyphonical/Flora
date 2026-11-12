<script lang="ts">
  import {
    Search,
    Plus,
    Upload,
    ArrowUpDown,
    Menu,
    PanelRightOpen,
    PanelRightClose,
    X
  } from '@lucide/svelte';
  import { store } from '../lib/store.svelte';

  let searchEl: HTMLInputElement | undefined = $state();
  let filePickerEl: HTMLInputElement | undefined = $state();

  function handleGlobalKey(e: KeyboardEvent) {
    if (
      e.key === '/' &&
      document.activeElement?.tagName !== 'INPUT' &&
      document.activeElement?.tagName !== 'TEXTAREA'
    ) {
      e.preventDefault();
      searchEl?.focus();
    }
  }

  async function handlePickedFiles(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    if (!input.files?.length) return;
    await store.uploadBrowserFiles(Array.from(input.files));
    input.value = '';
  }
</script>

<svelte:window onkeydown={handleGlobalKey} />

<header class="topbar">
  <div class="topbar-left">
    <button
      class="btn-icon mobile-menu-btn"
      onclick={() => (store.mobileNavOpen = true)}
      aria-label="Open navigation"
    >
      <Menu size={18} />
    </button>

    <div class="search-box">
      <Search size={14} class="search-icon" />
      <input
        bind:this={searchEl}
        type="text"
        placeholder="Search files and text…"
        value={store.search}
        oninput={(e) => {
          store.setSearch(e.currentTarget.value);
          if (store.tab !== 'files') store.tab = 'files';
        }}
      />
      {#if store.search}
        <button
          class="clear-btn"
          onclick={() => store.setSearch('')}
          aria-label="Clear search"
        >
          <X size={13} />
        </button>
      {:else}
        <span class="kbd">/</span>
      {/if}
    </div>
  </div>

  <div class="topbar-actions">
    <button class="btn" onclick={() => store.createNewPaste()}>
      <Plus size={14} />
      <span class="btn-label">New paste</span>
    </button>

    <button
      class="btn"
      onclick={() => {
        if (!store.dockVisible) {
          store.dockVisible = true;
          store.dockExpanded = true;
        } else {
          store.dockExpanded = !store.dockExpanded;
        }
      }}
    >
      <ArrowUpDown size={14} />
      <span class="btn-label">Transfers</span>
      {#if store.activeTransfers.length > 0}
        <span class="badge shared">{store.activeTransfers.length}</span>
      {/if}
    </button>

    <input
      bind:this={filePickerEl}
      type="file"
      multiple
      class="sr-only"
      onchange={handlePickedFiles}
    />

    <button class="btn btn-primary" onclick={() => filePickerEl?.click()}>
      <Upload size={14} strokeWidth={2.1} />
      <span>Upload</span>
    </button>

    {#if store.tab === 'files'}
      <button
        class="btn-icon desktop-inspector-toggle"
        title={store.rightCollapsed ? 'Show details' : 'Hide details'}
        aria-label={store.rightCollapsed ? 'Show details panel' : 'Hide details panel'}
        aria-pressed={!store.rightCollapsed}
        onclick={() => (store.rightCollapsed = !store.rightCollapsed)}
      >
        {#if store.rightCollapsed}
          <PanelRightOpen size={16} />
        {:else}
          <PanelRightClose size={16} />
        {/if}
      </button>
    {/if}
  </div>
</header>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 22px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }

  .topbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 0 9px;
    height: 32px;
    max-width: 460px;
    width: 100%;
    transition: border-color 120ms;
  }

  .search-box:focus-within {
    border-color: var(--accent);
  }

  :global(.search-icon) {
    color: var(--muted);
    flex-shrink: 0;
  }

  .search-box input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    color: var(--fg);
    font-size: 12.5px;
  }

  .search-box input::placeholder {
    color: var(--muted);
  }

  .clear-btn {
    color: var(--muted);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
  }

  .clear-btn:hover {
    color: var(--head);
  }

  .mobile-menu-btn {
    display: none;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  @media (max-width: 1024px) {
    .desktop-inspector-toggle {
      display: none;
    }
  }

  @media (max-width: 768px) {
    .mobile-menu-btn {
      display: inline-grid;
    }

    .btn-label {
      display: none;
    }
  }
</style>
