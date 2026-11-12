<script lang="ts">
  import { Download, Copy, Check, Folder, FileText, ChevronRight, Home } from '@lucide/svelte';
  import { store, formatBytes, shortHash, supportsThumbnail } from '../lib/store.svelte';

  let copiedCmd = $state(false);

  const info = $derived(store.shareInfo);
  const isDir = $derived(Boolean(info?.directory));
  const entries = $derived(info?.entries ?? []);
  const file = $derived(info?.entry ?? null);
  const totalBytes = $derived(entries.reduce((sum, e) => sum + e.size, 0));

  const shareBase = $derived(
    store.shareSubpath ? `/t/${store.shareToken}/${store.shareSubpath}` : `/t/${store.shareToken}`
  );
  const downloadUrl = $derived(`${shareBase}?download=1`);
  const previewUrl = $derived(`${shareBase}?raw=1&preview=1`);

  const cliCommand = $derived(
    `flora down ${typeof window !== 'undefined' ? window.location.origin : ''}${shareBase} -o ./${info?.root_name ?? 'flora'}`
  );

  function copyCli() {
    void navigator.clipboard?.writeText(cliCommand);
    copiedCmd = true;
    window.setTimeout(() => (copiedCmd = false), 1800);
  }

  function openEntry(entry: (typeof entries)[number]) {
    if (entry.directory) {
      void store.openPublicShare(store.shareToken, entry.path, true);
    }
  }

  function entryDownloadUrl(entry: (typeof entries)[number]): string {
    return `/t/${store.shareToken}/${entry.path}?download=1`;
  }

  function entryPreviewUrl(entry: (typeof entries)[number]): string {
    return `/t/${store.shareToken}/${entry.path}?raw=1&preview=1`;
  }

  function fileKind(path: string): 'image' | 'audio' | 'pdf' | 'text' {
    const ext = path.split('.').pop()?.toLowerCase() ?? '';
    if (['jpg', 'jpeg', 'png', 'webp', 'gif', 'svg'].includes(ext)) return 'image';
    if (['mp3', 'wav', 'flac', 'ogg', 'm4a'].includes(ext)) return 'audio';
    if (ext === 'pdf') return 'pdf';
    return 'text';
  }
</script>

<div class="standalone-stage">
  <div class="public-share-shell">
    <header class="share-top">
      <div class="brand-row">
        <img src="/branding/icon.svg" alt="Flora" class="share-logo" />
        <span class="brand-word">Flora</span>
        <span class="badge shared">{isDir ? 'shared folder' : 'shared file'}</span>
      </div>

      {#if store.shareLoading}
        <div class="share-status mono">Loading shared content…</div>
      {:else if store.shareError}
        <div class="share-status error">{store.shareError}</div>
      {:else if info}
        <div class="share-header-main">
          <div>
            {#if store.shareSubpath}
              <nav class="share-breadcrumb" aria-label="Breadcrumb">
                <button
                  class="crumb"
                  onclick={() => store.openPublicShare(store.shareToken, '', true)}
                >
                  <Home size={12} />
                  <span>{info.root_name}</span>
                </button>
                {#each store.shareSubpath.split('/').filter(Boolean) as part, idx (idx)}
                  <ChevronRight size={12} />
                  <span class="crumb current">{part}</span>
                {/each}
              </nav>
            {/if}
            <h1 class="editorial-title">
              {isDir ? (store.shareSubpath.split('/').pop() || info.root_name) : file?.name}
            </h1>
            <div class="page-meta-line">
              {#if isDir}
                <span>{entries.length} {entries.length === 1 ? 'entry' : 'entries'}</span>
                <span>·</span>
                <span class="mono">{formatBytes(totalBytes)}</span>
              {:else if file}
                <span class="mono">{formatBytes(file.size)}</span>
                {#if file.hash}
                  <span>·</span>
                  <span class="mono" title={file.hash}>{shortHash(file.hash)}</span>
                {/if}
              {/if}
            </div>
          </div>

          {#if !isDir && file}
            <a class="btn btn-primary" href={downloadUrl} download>
              <Download size={14} />
              <span>Download</span>
            </a>
          {/if}
        </div>
      {/if}
    </header>

    {#if info && isDir}
      <div class="ledger">
        <div class="ledger-head share-cols">
          <span>Name</span>
          <span style="text-align:right;">Size</span>
          <span class="col-hash">Checksum</span>
          <span style="text-align:right;">Action</span>
        </div>

        {#each entries as item (item.path)}
          <div class="ledger-row share-cols">
            <div class="name-cell">
              {#if item.directory}
                <Folder size={15} class="dir-icon" />
              {:else}
                <FileText size={15} class="file-icon" />
              {/if}
              <button
                class="file-name link"
                onclick={() => openEntry(item)}
              >
                {item.name}{item.directory ? '/' : ''}
              </button>
            </div>
            <div class="mono size-cell">
              {item.directory ? `${item.item_count ?? 0} items` : formatBytes(item.size)}
            </div>
            <div class="mono hash-cell col-hash">{shortHash(item.hash)}</div>
            <div class="dl-cell">
              {#if item.directory}
                <button class="btn btn-sm" onclick={() => openEntry(item)}>
                  <span>Open</span>
                </button>
              {:else}
                <a class="btn btn-sm" href={entryDownloadUrl(item)} download>
                  <Download size={12} />
                  <span>Download</span>
                </a>
              {/if}
            </div>
          </div>
        {:else}
          <div class="ledger-row empty-row">This shared folder is empty.</div>
        {/each}
      </div>
    {:else if info && file}
      <div class="file-preview-card">
        {#if fileKind(file.path) === 'image'}
          {#if supportsThumbnail(file.path, file.size)}
            <img src={previewUrl} alt={file.name} />
          {:else}
            <img src={previewUrl} alt={file.name} />
          {/if}
        {:else if fileKind(file.path) === 'audio'}
          <!-- svelte-ignore a11y_media_has_caption -->
          <audio controls src={previewUrl}></audio>
        {:else if fileKind(file.path) === 'pdf'}
          <iframe src={previewUrl} title={file.name}></iframe>
        {:else}
          <pre class="mono text-preview">{store.shareTextPreview ?? ''}</pre>
        {/if}
      </div>
    {/if}

    <div class="cli-bar">
      <code class="mono">
        <span class="accent">flora down</span> {cliCommand.replace(/^flora down /, '')}
      </code>
      <button class="btn-icon" onclick={copyCli} title="Copy command">
        {#if copiedCmd}
          <Check size={14} />
        {:else}
          <Copy size={14} />
        {/if}
      </button>
    </div>
  </div>
</div>

<style>
  .standalone-stage {
    min-height: 100%;
    display: grid;
    place-items: center;
    padding: 24px 16px;
    background: radial-gradient(circle at 50% 0%, #1f211b 0%, var(--bg) 52%);
  }

  .public-share-shell {
    width: 100%;
    max-width: 760px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .share-top {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
  }

  .brand-row {
    display: flex;
    align-items: center;
    gap: 9px;
  }

  .share-logo {
    width: 26px;
    height: 26px;
    border-radius: 6px;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 22px;
    color: var(--head);
    transform: translateY(1px);
  }

  .share-status {
    font-size: 12.5px;
    color: var(--dim);
  }

  .share-status.error {
    color: #e69d79;
  }

  .share-breadcrumb {
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
  }

  .crumb.current {
    color: var(--fg);
  }

  .share-header-main {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 14px;
    flex-wrap: wrap;
  }

  .share-cols {
    display: grid;
    grid-template-columns: minmax(180px, 1fr) 92px 132px 112px;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
  }

  .name-cell {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  :global(.dir-icon) {
    color: var(--accent);
    flex-shrink: 0;
  }

  :global(.file-icon) {
    color: var(--dim);
    flex-shrink: 0;
  }

  .file-name {
    color: var(--fg);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }

  .file-name.link:hover {
    color: var(--accent);
    text-decoration: underline;
  }

  .size-cell {
    text-align: right;
    color: var(--dim);
    font-size: 12px;
  }

  .hash-cell {
    color: var(--muted);
    font-size: 11.5px;
  }

  .dl-cell {
    display: flex;
    justify-content: flex-end;
  }

  .empty-row {
    padding: 22px 14px;
    color: var(--dim);
    font-size: 12.5px;
  }

  .file-preview-card {
    background: var(--surface-0);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 140px;
  }

  .file-preview-card img {
    max-width: 100%;
    max-height: 420px;
    object-fit: contain;
  }

  .file-preview-card audio {
    width: 100%;
    padding: 18px;
  }

  .file-preview-card iframe {
    width: 100%;
    height: 420px;
    border: none;
  }

  .text-preview {
    margin: 0;
    padding: 14px;
    width: 100%;
    max-height: 420px;
    overflow: auto;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--fg);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .cli-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    background: #121212;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 8px 12px;
    font-size: 12px;
    color: var(--dim);
  }

  .cli-bar code {
    overflow-x: auto;
    white-space: nowrap;
  }

  .accent {
    color: var(--accent);
  }

  @media (max-width: 640px) {
    .public-share-shell {
      padding: 16px;
    }

    .share-cols {
      grid-template-columns: minmax(0, 1fr) 76px 98px;
      padding: 9px 10px;
    }

    .col-hash {
      display: none;
    }
  }
</style>
