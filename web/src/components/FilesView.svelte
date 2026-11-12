<script lang="ts">
  import { untrack } from 'svelte';
  import {
    FolderPlus,
    FolderOpen,
    Download,
    Copy,
    Trash2,
    LayoutList,
    LayoutGrid,
    ChevronRight,
    Home,
    ArrowLeft,
    Save,
    Share2,
    Info,
    PencilLine,
    ExternalLink
  } from '@lucide/svelte';
  import {
    store,
    formatBytes,
    shortHash,
    supportsThumbnail,
    type ConflictMode,
    type FileItem
  } from '../lib/store.svelte';
  import CodeEditor from '../lib/components/CodeEditor.svelte';
  import FileIcon from './FileIcon.svelte';

  let creatingFolder = $state(false);
  let newFolderName = $state('');
  let dragActive = $state(false);

  // Context menu & quick rename state
  let ctxMenu = $state<{ item: FileItem; x: number; y: number } | null>(null);
  let renamingItem = $state<FileItem | null>(null);
  let renameDraft = $state('');
  // Inline text file editor state (when opening a text file inside Files)
  let editorPath = $state('');
  let editorContent = $state('');
  let editorLoading = $state(false);

  const editingItem = $derived(store.editingFile);
  const items = $derived(store.visibleFiles);
  const totalFolderBytes = $derived(items.reduce((sum, f) => sum + f.bytes, 0));
  const isSearching = $derived(store.search.trim().length > 0);

  const folderHeading = $derived(
    isSearching
      ? `Search "${store.search.trim()}"`
      : store.currentFolder
        ? store.currentFolder.split('/').pop()!
        : 'Files'
  );

  const editorLineCount = $derived(Math.max(1, (editorContent || '').split('\n').length));

  let loadedEditorPath = '';

  $effect(() => {
    const path = editingItem?.path;
    if (!path || path === loadedEditorPath) return;
    loadedEditorPath = path;
    editorPath = path;
    editorLoading = true;
    let cancelled = false;
    void untrack(() => store.ensureTextLoaded(path)).then((content) => {
      if (cancelled) return;
      editorContent = content;
      editorLoading = false;
    });
    return () => {
      cancelled = true;
    };
  });

  function handleRowClick(f: FileItem) {
    store.selectFile(f);
  }

  async function handleRowDoubleClick(f: FileItem) {
    if (f.directory) {
      await store.openFolder(f.path);
    } else if (f.kind === 'text') {
      await store.openEditor(f);
    }
  }

  function submitNewFolder() {
    if (!newFolderName.trim()) return;
    void store.createFolder(newFolderName);
    newFolderName = '';
    creatingFolder = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragActive = false;
    const dropped = e.dataTransfer?.files;
    if (dropped && dropped.length > 0) {
      await store.uploadBrowserFiles(Array.from(dropped));
    }
  }

  function openContextMenu(e: MouseEvent, f: FileItem) {
    e.preventDefault();
    e.stopPropagation();
    store.selectFile(f);
    const menuW = 204;
    const menuH = 228;
    const x = Math.max(8, Math.min(e.clientX, window.innerWidth - menuW - 8));
    const y = Math.max(8, Math.min(e.clientY, window.innerHeight - menuH - 8));
    ctxMenu = { item: f, x, y };
  }

  function closeContextMenu() {
    if (ctxMenu) ctxMenu = null;
  }

  function beginQuickRename(f: FileItem) {
    renamingItem = f;
    renameDraft = f.path;
    ctxMenu = null;
  }

  function submitQuickRename() {
    if (!renamingItem) return;
    void store.renameFile(renamingItem.path, renameDraft);
    renamingItem = null;
  }

  function copyShareLink(f: FileItem) {
    if (!f.sharedToken) return;
    void navigator.clipboard?.writeText(store.shareUrlFor(f.sharedToken));
    store.notify('Copied share link');
    ctxMenu = null;
  }
</script>

<svelte:window
  onclick={closeContextMenu}
  onscroll={closeContextMenu}
  onkeydown={(e) => {
    if (e.key === 'Escape') {
      ctxMenu = null;
      renamingItem = null;
    }
  }}
/>

{#if editingItem}
  <div class="editor-shell">
    <div class="page-header">
      <div>
        <button class="back-crumb" onclick={() => store.closeEditor()}>
          <ArrowLeft size={14} />
          <span>Back to /{store.currentFolder || ''}</span>
        </button>
        <h1 class="editorial-title">{editingItem.name}</h1>
      </div>

      <div class="header-controls">
        <button class="btn" onclick={() => store.toggleShareLink(editingItem)}>
          <Share2 size={14} />
          <span>{editingItem.sharedToken ? 'Shared' : 'Share'}</span>
        </button>
        <button
          class="btn btn-primary"
          onclick={() => store.saveFileContent(editingItem.path, editorPath, editorContent)}
        >
          <Save size={14} />
          <span>Save</span>
        </button>
      </div>
    </div>

    <div class="file-editor-card">
      <div class="file-editor-toolbar">
        <div class="toolbar-left">
          <span class="mono path-prefix">/</span>
          <input
            class="input mono path-input"
            value={editorPath}
            oninput={(e) => (editorPath = e.currentTarget.value)}
            aria-label="File path"
          />
        </div>
        <span class="mono editor-stats">
          {editorLineCount} lines · {formatBytes(new TextEncoder().encode(editorContent).length)}
        </span>
      </div>

      {#if editorLoading}
        <div class="editor-loading mono">Loading file…</div>
      {:else}
        <CodeEditor bind:code={editorContent} filename={editingItem.name} />
      {/if}
    </div>
  </div>
{:else}
  <div
    class="files-stage"
    role="region"
    aria-label="File browser"
    ondragover={(e) => {
      e.preventDefault();
      dragActive = true;
    }}
    ondragleave={() => (dragActive = false)}
    ondrop={handleDrop}
  >
    <div class="page-header">
      <div>
        <nav class="breadcrumb-trail" aria-label="Breadcrumb">
          <button
            class="crumb"
            class:current={!store.currentFolder}
            onclick={() => store.openFolder('')}
          >
            <Home size={12} />
            <span>~</span>
          </button>
          {#each store.breadcrumbs as crumb, idx (crumb.path)}
            <ChevronRight size={12} class="crumb-sep" />
            <button
              class="crumb"
              class:current={idx === store.breadcrumbs.length - 1}
              onclick={() => store.openFolder(crumb.path)}
            >
              {crumb.name}
            </button>
          {/each}
        </nav>

        <h1 class="editorial-title">{folderHeading}</h1>
        <div class="page-meta-line">
          <span>{items.length} {items.length === 1 ? 'item' : 'items'}</span>
          <span>·</span>
          <span class="mono">{formatBytes(totalFolderBytes)}</span>
        </div>
      </div>

      <div class="header-controls">
        <label class="conflict-pill">
          <span>On conflict</span>
          <select
            value={store.conflictMode}
            onchange={(e) => (store.conflictMode = e.currentTarget.value as ConflictMode)}
          >
            <option value="reject">Reject</option>
            <option value="rename">Rename</option>
            <option value="replace">Replace</option>
          </select>
        </label>

        <button class="btn btn-sm" onclick={() => (creatingFolder = !creatingFolder)}>
          <FolderPlus size={14} />
          <span>New folder</span>
        </button>

        <div class="segmented" role="group" aria-label="View mode">
          <button
            class:active={store.viewMode === 'list'}
            onclick={() => (store.viewMode = 'list')}
          >
            <LayoutList size={13} />
            <span>List</span>
          </button>
          <button
            class:active={store.viewMode === 'grid'}
            onclick={() => (store.viewMode = 'grid')}
          >
            <LayoutGrid size={13} />
            <span>Grid</span>
          </button>
        </div>
      </div>
    </div>

    {#if creatingFolder}
      <div class="new-folder-bar">
        <input
          class="input"
          placeholder="Folder name"
          value={newFolderName}
          oninput={(e) => (newFolderName = e.currentTarget.value)}
          onkeydown={(e) => e.key === 'Enter' && submitNewFolder()}
        />
        <button class="btn btn-primary btn-sm" onclick={submitNewFolder}>Create</button>
        <button class="btn btn-sm" onclick={() => (creatingFolder = false)}>Cancel</button>
      </div>
    {/if}
    {#if renamingItem}
      <div class="new-folder-bar">
        <span class="mono path-prefix">Move / rename:</span>
        <input
          class="input mono"
          style="flex:1;max-width:340px;"
          value={renameDraft}
          oninput={(e) => (renameDraft = e.currentTarget.value)}
          onkeydown={(e) => e.key === 'Enter' && submitQuickRename()}
        />
        <button class="btn btn-primary btn-sm" onclick={submitQuickRename}>Save</button>
        <button class="btn btn-sm" onclick={() => (renamingItem = null)}>Cancel</button>
      </div>
    {/if}

    {#if dragActive}
      <div class="drop-active-banner">Drop files to upload to /{store.currentFolder || ''}</div>
    {/if}

    {#if items.length === 0}
      <div class="empty-folder">
        <p>{isSearching ? 'No matching files.' : 'No matching files in this directory.'}</p>
      </div>
    {:else if store.viewMode === 'list'}
      <div class="ledger">
        <div class="ledger-head files-cols">
          <span>Name</span>
          <span class="col-right">Size</span>
          <span class="col-hash">Checksum</span>
          <span class="col-date col-right">Modified</span>
          <span class="col-mobile-act"></span>
        </div>

        {#each items as f (f.path)}
          <div
            class="ledger-row interactive files-cols"
            class:is-folder={f.directory}
            class:selected={store.selectedFile?.path === f.path}
            role="button"
            tabindex="0"
            onclick={() => handleRowClick(f)}
            ondblclick={() => handleRowDoubleClick(f)}
            oncontextmenu={(e) => openContextMenu(e, f)}
            onkeydown={(e) => {
              if (e.key === 'Enter') handleRowDoubleClick(f);
            }}
          >
            <div class="cell-name">
              <FileIcon kind={f.kind} />
              <div class="file-title-wrap">
                {#if f.directory}
                  <button
                    class="folder-link"
                    onclick={(e) => {
                      e.stopPropagation();
                      void store.openFolder(f.path);
                    }}
                  >
                    {f.name}/
                  </button>
                {:else}
                  <span class="file-title">{f.name}</span>
                {/if}

                {#if f.sharedToken}
                  <span class="badge shared">shared</span>
                {/if}
              </div>
            </div>

            <div class="cell-num">
              {f.directory ? `${f.itemCount ?? 0} items` : formatBytes(f.bytes)}
            </div>

            <div class="cell-hash col-hash">{shortHash(f.hash)}</div>

            <div class="cell-date col-date">{f.modified}</div>

            <div class="col-mobile-act">
              {#if f.kind === 'text'}
                <button
                  class="btn-icon"
                  title="Edit file"
                  onclick={(e) => {
                    e.stopPropagation();
                    void store.openEditor(f);
                  }}
                >
                  <PencilLine size={14} />
                </button>
              {/if}
              <button
                class="btn-icon mobile-info-btn"
                title="Details"
                onclick={(e) => {
                  e.stopPropagation();
                  store.selectFile(f);
                  store.mobileInspectorOpen = true;
                }}
              >
                <Info size={15} />
              </button>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="thumb-grid">
        {#each items as f (f.path)}
          <div
            class="thumb-card"
            class:selected={store.selectedFile?.path === f.path}
            role="button"
            tabindex="0"
            onclick={() => handleRowClick(f)}
            ondblclick={() => handleRowDoubleClick(f)}
            oncontextmenu={(e) => openContextMenu(e, f)}
            onkeydown={(e) => {
              if (e.key === 'Enter') handleRowDoubleClick(f);
            }}
          >
            <div class="thumb-preview">
              {#if f.kind === 'image' && supportsThumbnail(f.path, f.bytes)}
                <img
                  src={store.thumbnailUrlFor(f.path)}
                  alt={f.name}
                  loading="lazy"
                />
              {:else}
                <FileIcon kind={f.kind} size={18} />
              {/if}

              {#if f.sharedToken}
                <span class="badge shared grid-badge">shared</span>
              {/if}
            </div>

            <div class="thumb-meta">
              <div class="thumb-name" class:folder-text={f.directory}>
                {f.name}{f.directory ? '/' : ''}
              </div>
              <div class="thumb-sub mono">
                <span>{f.directory ? `${f.itemCount ?? 0} items` : formatBytes(f.bytes)}</span>
                <span>{f.modified.split(',')[0]}</span>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

{#if ctxMenu}
  {@const f = ctxMenu.item}
  <div
    class="context-menu"
    role="menu"
    tabindex="-1"
    style:left="{ctxMenu.x}px"
    style:top="{ctxMenu.y}px"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.stopPropagation()}
  >
    {#if f.directory}
      <button
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          void store.openFolder(f.path);
          ctxMenu = null;
        }}
      >
        <FolderOpen size={14} />
        <span>Open folder</span>
      </button>
    {:else}
      {#if f.kind === 'text'}
        <button
          class="ctx-item"
          role="menuitem"
          onclick={() => {
            void store.openEditor(f);
            ctxMenu = null;
          }}
        >
          <PencilLine size={14} />
          <span>Edit file</span>
        </button>
      {/if}
      <button
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          store.triggerDownload(f.path);
          ctxMenu = null;
        }}
      >
        <Download size={14} />
        <span>Download</span>
      </button>
    {/if}

    <button
      class="ctx-item"
      role="menuitem"
      onclick={() => {
        void store.toggleShareLink(f);
        ctxMenu = null;
      }}
    >
      <Share2 size={14} />
      <span>{f.sharedToken ? 'Revoke share link' : 'Create share link'}</span>
    </button>

    {#if f.sharedToken}
      <button
        class="ctx-item"
        role="menuitem"
        onclick={() => copyShareLink(f)}
      >
        <Copy size={14} />
        <span>Copy share link</span>
      </button>
      <button
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          window.open(store.shareUrlFor(f.sharedToken!), '_blank', 'noopener');
          ctxMenu = null;
        }}
      >
        <ExternalLink size={14} />
        <span>Open share page</span>
      </button>
    {/if}

    {#if !f.directory}
      <button
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (f.hash && f.hash !== '—') {
            void navigator.clipboard?.writeText(f.hash);
            store.notify('Copied XXH3-128 checksum');
          }
          ctxMenu = null;
        }}
      >
        <Copy size={14} />
        <span>Copy checksum</span>
      </button>
    {/if}

    <button
      class="ctx-item"
      role="menuitem"
      onclick={() => beginQuickRename(f)}
    >
      <PencilLine size={14} />
      <span>Move / Rename…</span>
    </button>

    <div class="ctx-sep"></div>

    <button
      class="ctx-item danger"
      role="menuitem"
      onclick={() => {
        void store.moveToTrash(f.path);
        ctxMenu = null;
      }}
    >
      <Trash2 size={14} />
      <span>Move to trash</span>
    </button>
  </div>
{/if}

<style>
  .files-stage {
    min-height: 100%;
  }

  .breadcrumb-trail {
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--dim);
    margin-bottom: 6px;
    flex-wrap: wrap;
  }

  .crumb {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--dim);
    padding: 2px 5px;
    border-radius: 4px;
  }

  .crumb:hover {
    color: var(--accent);
    background: var(--surface-1);
  }

  .crumb.current {
    color: var(--fg);
  }

  :global(.crumb-sep) {
    color: var(--muted);
  }

  .conflict-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 4px 9px;
  }

  .conflict-pill select {
    background: transparent;
    border: none;
    outline: none;
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: 11.5px;
    cursor: pointer;
  }

  .conflict-pill select option {
    background: var(--surface-2);
    color: var(--fg);
  }

  .new-folder-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
    padding: 10px 12px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
  }

  .drop-active-banner {
    border: 1px dashed var(--accent);
    background: var(--accent-soft);
    color: var(--head);
    border-radius: var(--r-md);
    padding: 12px 14px;
    margin-bottom: 14px;
    font-size: 12.5px;
    text-align: center;
  }

  .empty-folder {
    padding: 42px 20px;
    text-align: center;
    color: var(--dim);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    background: var(--surface-1);
  }

  .files-cols {
    display: grid;
    grid-template-columns: minmax(200px, 1fr) 92px 128px 128px 32px;
    align-items: center;
    gap: 12px;
    padding: 8.5px 14px;
  }

  .col-right {
    text-align: right;
  }

  .cell-name {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .file-title-wrap {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .file-title,
  .folder-link {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--fg);
    font-weight: 500;
  }

  .folder-link {
    color: var(--accent);
    text-align: left;
  }

  .folder-link:hover {
    text-decoration: underline;
  }

  .ledger-row.selected .file-title {
    color: var(--head);
  }

  .cell-num {
    font-family: var(--font-mono);
    font-variant-ligatures: none;
    font-size: 12px;
    color: var(--dim);
    text-align: right;
  }

  .cell-hash {
    font-family: var(--font-mono);
    font-variant-ligatures: none;
    font-size: 11.5px;
    color: var(--muted);
  }

  .cell-date {
    font-size: 12px;
    color: var(--dim);
    text-align: right;
  }

  .col-mobile-act {
    display: flex;
    justify-content: flex-end;
    align-items: center;
  }

  .mobile-info-btn {
    display: none;
  }

  /* Grid view */
  .thumb-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 12px;
  }

  .thumb-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    overflow: hidden;
    cursor: pointer;
    transition: border-color 120ms;
  }

  .thumb-card:hover {
    border-color: var(--border-hover);
  }

  .thumb-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .thumb-preview {
    height: 118px;
    background: #161616;
    border-bottom: 1px solid var(--border);
    display: grid;
    place-items: center;
    position: relative;
    overflow: hidden;
  }

  .thumb-preview img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .grid-badge {
    position: absolute;
    top: 8px;
    right: 8px;
  }

  .thumb-meta {
    padding: 9px 11px;
  }

  .thumb-name {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .thumb-name.folder-text {
    color: var(--accent);
  }

  .thumb-sub {
    font-size: 11px;
    color: var(--dim);
    margin-top: 3px;
    display: flex;
    justify-content: space-between;
  }

  /* Inline File Editor (JetBrains Mono) */
  .back-crumb {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--dim);
    margin-bottom: 6px;
  }

  .back-crumb:hover {
    color: var(--accent);
  }

  .file-editor-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .file-editor-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
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
    min-width: 200px;
  }

  .path-prefix {
    color: var(--dim);
    font-size: 13px;
  }

  .path-input {
    width: 100%;
    max-width: 340px;
  }

  .editor-stats {
    font-size: 11.5px;
    color: var(--dim);
  }

  .editor-loading {
    padding: 18px 14px;
    color: var(--dim);
    font-size: 12px;
  }

  .context-menu {
    position: fixed;
    z-index: 70;
    min-width: 196px;
    background: var(--surface-2);
    border: 1px solid var(--border-hover);
    border-radius: var(--r-md);
    padding: 4px;
    box-shadow: 0 14px 34px rgba(0, 0, 0, 0.58);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .ctx-item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 6.5px 10px;
    border-radius: var(--r-sm);
    font-size: 12.5px;
    color: var(--fg);
    text-align: left;
    transition: background 100ms, color 100ms;
  }

  .ctx-item:hover {
    background: var(--surface-3);
    color: var(--head);
  }

  .ctx-item.danger {
    color: #e69d79;
  }

  .ctx-item.danger:hover {
    background: var(--danger-soft);
    color: var(--head);
  }

  .ctx-sep {
    height: 1px;
    background: var(--border);
    margin: 3px 4px;
  }

  @media (max-width: 1024px) {
    .mobile-info-btn {
      display: inline-grid;
    }

    .files-cols {
      grid-template-columns: minmax(180px, 1fr) 86px 118px 64px;
    }

    .col-hash {
      display: none;
    }
  }

  @media (max-width: 640px) {
    .files-cols {
      grid-template-columns: minmax(0, 1fr) 78px 62px;
      padding: 8.5px 10px;
      gap: 8px;
    }

    .col-date {
      display: none;
    }

    .conflict-pill span {
      display: none;
    }
  }
</style>
