<script lang="ts">
  import {
    Download,
    Share2,
    PencilLine,
    Trash2,
    Copy,
    Check,
    FolderOpen,
    X,
    Link2,
    Plus,
    ExternalLink
  } from '@lucide/svelte';
  import {
    store,
    formatBytes,
    shortHash,
    supportsThumbnail,
    type FileItem
  } from '../lib/store.svelte';
  import Highlighted from '../lib/components/Highlighted.svelte';
  import FileIcon from './FileIcon.svelte';

  let { mobile = false }: { mobile?: boolean } = $props();
  let copied = $state(false);
  let renaming = $state(false);
  let renameDraft = $state('');

  const item = $derived(store.editingFile ?? store.selectedFile);
  const textContent = $derived(item ? store.getCachedText(item.path) ?? '' : '');
  const previewUrl = $derived(item && !item.directory ? store.previewUrlFor(item.path) : '');

  function copyLink(token: string) {
    void navigator.clipboard?.writeText(store.shareUrlFor(token));
    copied = true;
    store.notify('Copied link to clipboard');
    window.setTimeout(() => (copied = false), 1800);
  }

  function startRename() {
    if (!item) return;
    renameDraft = item.path;
    renaming = true;
  }

  function commitRename() {
    if (!item) return;
    void store.renameFile(item.path, renameDraft);
    renaming = false;
  }

  function isText(item: FileItem): boolean {
    return item.kind === 'text';
  }
</script>

<aside class="inspector" class:mobile>
  <div class="inspector-top">
    <span class="inspector-heading">Details</span>
    {#if mobile}
      <button
        class="btn-icon"
        onclick={() => (store.mobileInspectorOpen = false)}
        aria-label="Close details"
      >
        <X size={16} />
      </button>
    {/if}
  </div>

  {#if item}
    <div class="inspector-preview">
      <div class="preview-stage">
        {#if item.directory}
          <div class="generic-stage">
            <FileIcon kind="folder" size={20} />
            <span class="mono stage-ext">{item.itemCount ?? 0} items</span>
          </div>
        {:else if item.kind === 'image'}
          {#if supportsThumbnail(item.path, item.bytes)}
            <img class="image-preview" src={store.thumbnailUrlFor(item.path)} alt={item.name} />
          {:else}
            <img class="image-preview" src={previewUrl} alt={item.name} />
          {/if}
        {:else if item.kind === 'audio'}
          <audio class="media-preview" controls src={previewUrl}>
            <track kind="captions" />
          </audio>
        {:else if item.kind === 'pdf'}
          <iframe class="pdf-preview" src={previewUrl} title={item.name}></iframe>
        {:else if isText(item)}
          <Highlighted code={textContent} filename={item.name} />
        {:else}
          <div class="generic-stage">
            <FileIcon kind={item.kind} size={20} />
            <span class="mono stage-ext">
              {item.name.split('.').pop()?.toUpperCase()}
            </span>
          </div>
        {/if}
      </div>

      <div class="inspector-title-box">
        <div class="inspector-title">{item.name}</div>
        <div class="inspector-path mono">/{item.path}</div>
      </div>
    </div>

    {#if renaming}
      <div class="kv-card">
        <div class="card-heading"><span>Move / Rename</span></div>
        <input
          class="input mono"
          value={renameDraft}
          oninput={(e) => (renameDraft = e.currentTarget.value)}
          onkeydown={(e) => e.key === 'Enter' && commitRename()}
        />
        <div class="inline-btns">
          <button class="btn btn-sm" onclick={() => (renaming = false)}>Cancel</button>
          <button class="btn btn-primary btn-sm" onclick={commitRename}>Save path</button>
        </div>
      </div>
    {/if}

    <div class="inspector-actions">
      {#if item.directory}
        <button
          class="btn btn-primary"
          onclick={() => {
            void store.openFolder(item.path);
            store.mobileInspectorOpen = false;
          }}
        >
          <FolderOpen size={14} />
          <span>Open</span>
        </button>
      {:else if isText(item)}
        {#if store.editingFilePath === item.path}
          <button class="btn" onclick={() => store.closeEditor()}>
            <PencilLine size={14} />
            <span>Close editor</span>
          </button>
        {:else}
          <button
            class="btn btn-primary"
            onclick={() => {
              void store.openEditor(item);
              store.mobileInspectorOpen = false;
            }}
          >
            <PencilLine size={14} />
            <span>Edit file</span>
          </button>
        {/if}
      {:else}
        <button
          class="btn btn-primary"
          onclick={() => store.triggerDownload(item.path)}
        >
          <Download size={14} />
          <span>Download</span>
        </button>
      {/if}

      <button class="btn" onclick={() => store.toggleShareLink(item)}>
        <Share2 size={14} />
        <span>{item.sharedToken ? 'Unshare' : 'Share'}</span>
      </button>

      <button class="btn" onclick={startRename}>
        <PencilLine size={14} />
        <span>Rename</span>
      </button>

      <button class="btn btn-danger" onclick={() => store.moveToTrash(item.path)}>
        <Trash2 size={14} />
        <span>Trash</span>
      </button>
    </div>

    <div class="kv-card">
      <div class="card-heading">
        <span>Properties</span>
      </div>
      <div class="kv-row">
        <span class="kv-key">Size</span>
        <span class="kv-val">{formatBytes(item.bytes)}</span>
      </div>
      <div class="kv-row">
        <span class="kv-key">Modified</span>
        <span class="kv-val">{item.modified}</span>
      </div>
      {#if item.directory}
        <div class="kv-row">
          <span class="kv-key">Items</span>
          <span class="kv-val">{item.itemCount ?? 0}</span>
        </div>
      {:else}
        <div class="kv-row">
          <span class="kv-key">XXH3-128</span>
          <span class="kv-val" title={item.hash}>{shortHash(item.hash)}</span>
        </div>
      {/if}
    </div>

    <div class="kv-card">
      <div class="card-heading">
        <span>Shared link</span>
        {#if !item.sharedToken}
          <button class="btn btn-sm" onclick={() => store.toggleShareLink(item)}>
            <Plus size={12} />
            <span>Create</span>
          </button>
        {/if}
      </div>

      {#if item.sharedToken}
        <div class="share-item">
          <div class="share-meta">
            <span class="share-kind">
              <Link2 size={13} />
              Read-only {item.directory ? 'folder' : 'file'}
            </span>
            <button
              class="revoke-link mono"
              onclick={() => store.toggleShareLink(item)}
            >
              Revoke
            </button>
          </div>
          <div class="share-url-box">
            <span>{store.shareUrlFor(item.sharedToken)}</span>
            <button
              class="copy-trigger"
              onclick={() => copyLink(item.sharedToken!)}
              title="Copy link"
            >
              {#if copied}
                <Check size={13} />
              {:else}
                <Copy size={13} />
              {/if}
            </button>
            <button
              class="copy-trigger"
              onclick={() => window.open(store.shareUrlFor(item.sharedToken!), '_blank', 'noopener')}
              title="Open share page"
            >
              <ExternalLink size={13} />
            </button>
          </div>
        </div>
      {:else}
        <div class="empty-note">Not shared</div>
      {/if}
    </div>
  {:else}
    <div class="empty-inspector">Select a file or folder to view details.</div>
  {/if}
</aside>

<style>
  .inspector {
    background: var(--surface-0);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 14px 16px 22px;
    gap: 14px;
    height: 100%;
    min-width: 0;
  }

  .inspector-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .inspector-heading {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.09em;
    color: var(--muted);
    font-weight: 600;
  }

  .inspector-preview {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  .preview-stage {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    overflow: hidden;
    min-height: 118px;
    max-height: 220px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .image-preview {
    width: 100%;
    max-height: 220px;
    object-fit: contain;
    display: block;
  }

  .media-preview,
  .pdf-preview {
    width: 100%;
    height: 190px;
    border: none;
    display: block;
  }

  .generic-stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 26px 10px;
  }

  .stage-ext {
    font-size: 10.5px;
    color: var(--muted);
    letter-spacing: 0.07em;
  }

  .inspector-title-box {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .inspector-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--head);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .inspector-path {
    font-size: 10.5px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .inspector-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
  }

  .kv-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 10px 11px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .card-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--muted);
    font-weight: 600;
  }

  .kv-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    font-size: 12px;
  }

  .kv-key {
    color: var(--dim);
  }

  .kv-val {
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: 11px;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .inline-btns {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .share-item {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .share-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .share-kind {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--dim);
  }

  .revoke-link {
    font-size: 10.5px;
    color: var(--muted);
  }

  .revoke-link:hover {
    color: var(--danger);
  }

  .share-url-box {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--surface-0);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 5px 7px;
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--dim);
  }

  .share-url-box span {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .copy-trigger {
    color: var(--muted);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .copy-trigger:hover {
    color: var(--head);
  }

  .empty-note,
  .empty-inspector {
    font-size: 12px;
    color: var(--muted);
  }

  .empty-inspector {
    padding: 30px 10px;
    text-align: center;
    border: 1px dashed var(--border);
    border-radius: var(--r-sm);
  }
</style>
