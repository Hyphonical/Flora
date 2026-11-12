<script lang="ts">
  import { KeyRound, Copy, Trash2, ExternalLink, Check, X } from '@lucide/svelte';
  import { store, shortHash, formatTimestamp } from '../lib/store.svelte';

  let showMint = $state(false);
  let clientLabel = $state('');
  let copiedSecret = $state(false);

  const list = $derived(store.ownerTokens);

  function submitToken() {
    void store.mintAccountToken(clientLabel);
    clientLabel = '';
    showMint = false;
  }

  function copySecret() {
    if (!store.mintedAccountToken) return;
    void navigator.clipboard?.writeText(store.mintedAccountToken.token);
    copiedSecret = true;
    window.setTimeout(() => (copiedSecret = false), 1800);
  }
</script>

<div>
  <div class="page-header">
    <div>
      <h1 class="editorial-title">Links &amp; Tokens</h1>
      <div class="page-meta-line">
        <span>{list.length} active credentials</span>
      </div>
    </div>

    <div class="header-controls">
      <button class="btn btn-primary" onclick={() => (showMint = !showMint)}>
        <KeyRound size={14} />
        <span>New CLI token</span>
      </button>
    </div>
  </div>

  {#if store.mintedAccountToken}
    <div class="mint-banner">
      <div class="mint-banner-head">
        <span class="badge warn">shown once</span>
        <span>
          Account token{store.mintedAccountToken.label
            ? ` “${store.mintedAccountToken.label}”`
            : ''} — copy it now, it is never shown again.
        </span>
        <button
          class="btn-icon"
          onclick={() => (store.mintedAccountToken = null)}
          aria-label="Dismiss token"
        >
          <X size={14} />
        </button>
      </div>
      <div class="secret-box">
        <code class="mono secret-text">{store.mintedAccountToken.token}</code>
        <button class="btn btn-sm" onclick={copySecret}>
          {#if copiedSecret}
            <Check size={12} />
            <span>Copied</span>
          {:else}
            <Copy size={12} />
            <span>Copy token</span>
          {/if}
        </button>
      </div>
    </div>
  {/if}

  {#if showMint}
    <div class="mint-bar">
      <input
        class="input"
        placeholder="Device name (e.g. thinkpad-x1)"
        value={clientLabel}
        oninput={(e) => (clientLabel = e.currentTarget.value)}
        onkeydown={(e) => e.key === 'Enter' && submitToken()}
      />
      <button class="btn btn-primary btn-sm" onclick={submitToken}>Create token</button>
      <button class="btn btn-sm" onclick={() => (showMint = false)}>Cancel</button>
    </div>
  {/if}

  <div class="ledger">
    <div class="ledger-head token-cols">
      <span>Scope</span>
      <span>Permission</span>
      <span class="col-digest">Digest</span>
      <span class="col-created">Created</span>
      <span style="text-align:right;">Actions</span>
    </div>

    {#each list as tok (tok.hash)}
      <div class="ledger-row token-cols">
        <div class="scope-cell">
          <span
            class="badge"
            class:shared={tok.kind === 'file' || tok.kind === 'folder'}
            class:warn={tok.kind === 'account'}
          >
            {tok.kind}
          </span>
          <span class="scope-name" class:mono={!tok.label}>
            {tok.label ? tok.label : `/${tok.scope}`}
          </span>
        </div>

        <div class="perm-cell mono">
          {#if tok.kind === 'file' || tok.kind === 'folder'}
            read-only
          {:else}
            read / write
          {/if}
        </div>

        <div class="mono digest-cell col-digest">{shortHash(tok.hash)}</div>

        <div class="created-cell col-created">{formatTimestamp(tok.created)}</div>

        <div class="act-cell">
          {#if tok.secret}
            <button
              class="btn btn-sm"
              title="Open public share page"
              onclick={() => window.open(store.shareUrlFor(tok.secret!), '_blank', 'noopener')}
            >
              <ExternalLink size={12} />
            </button>
            <button
              class="btn btn-sm"
              title="Copy link"
              onclick={() => {
                void navigator.clipboard?.writeText(store.shareUrlFor(tok.secret!));
                store.notify('Copied link to clipboard');
              }}
            >
              <Copy size={12} />
            </button>
          {/if}
          <button
            class="btn btn-danger btn-sm"
            onclick={() => store.revokeToken(tok.hash)}
          >
            <Trash2 size={12} />
            <span>Revoke</span>
          </button>
        </div>
      </div>
    {:else}
      <div class="ledger-row empty-row">No tokens yet.</div>
    {/each}
  </div>
</div>

<style>
  .mint-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
    padding: 10px 12px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    flex-wrap: wrap;
  }

  .mint-bar .input {
    min-width: 240px;
  }

  .mint-banner {
    margin-bottom: 14px;
    padding: 11px 12px;
    background: var(--surface-1);
    border: 1px solid rgba(194, 168, 108, 0.4);
    border-radius: var(--r-md);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  .mint-banner-head {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 12.5px;
    color: var(--head);
  }

  .mint-banner-head span:nth-child(2) {
    flex: 1;
  }

  .secret-box {
    display: flex;
    align-items: center;
    gap: 9px;
    background: var(--surface-0);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 7px 9px;
  }

  .secret-text {
    flex: 1;
    font-size: 11.5px;
    color: var(--accent);
    overflow-x: auto;
    white-space: nowrap;
  }

  .token-cols {
    display: grid;
    grid-template-columns: minmax(200px, 1.4fr) 105px 130px 90px 150px;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
  }

  .scope-cell {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .scope-name {
    color: var(--fg);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .perm-cell {
    font-size: 11.5px;
    color: var(--dim);
  }

  .digest-cell {
    font-size: 11.5px;
    color: var(--muted);
  }

  .created-cell {
    font-size: 12px;
    color: var(--dim);
  }

  .act-cell {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .empty-row {
    padding: 22px 14px;
    color: var(--dim);
    font-size: 12.5px;
  }

  @media (max-width: 900px) {
    .token-cols {
      grid-template-columns: minmax(160px, 1fr) 95px 130px;
    }

    .col-digest,
    .col-created {
      display: none;
    }
  }

  @media (max-width: 600px) {
    .token-cols {
      grid-template-columns: minmax(0, 1fr) 92px;
    }

    .act-cell {
      gap: 4px;
    }
  }
</style>
