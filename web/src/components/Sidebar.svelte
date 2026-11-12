<script lang="ts">
  import {
    Folder,
    FileCode2,
    KeyRound,
    Trash2,
    Server,
    ChevronsUpDown,
    PanelLeftClose,
    PanelLeftOpen,
    LogOut,
    X
  } from '@lucide/svelte';
  import { store, formatBytes, type Tab } from '../lib/store.svelte';

  let { mobile = false }: { mobile?: boolean } = $props();
  let spaceMenuOpen = $state(false);

  const collapsed = $derived(!mobile && store.leftCollapsed);
  const user = $derived(store.currentUser);
  const quotaBytes = $derived(user.quotaBytes ?? 0);
  const liveQuotaPct = $derived(
    quotaBytes > 0 ? Math.min(100, (user.usedBytes / quotaBytes) * 100) : 0
  );
  const trashQuotaPct = $derived(
    quotaBytes > 0 ? Math.min(100 - liveQuotaPct, (user.trashBytes / quotaBytes) * 100) : 0
  );
  const bufferBytes = $derived(store.totalBufferReserved);
  const bufferSize = $derived(store.dashboardBufferSize || store.server.buffer_size);
  const bufferPct = $derived(
    bufferSize > 0 ? Math.min(100, (bufferBytes / bufferSize) * 100) : 0
  );

  const navItems = $derived<
    { id: Tab; label: string; count: string; icon: typeof Folder }[]
  >([
    {
      id: 'files',
      label: 'Files',
      count: formatBytes(user.usedBytes),
      icon: Folder
    },
    {
      id: 'pastes',
      label: 'Pastes',
      count: String(store.pasteFiles.length),
      icon: FileCode2
    },
    {
      id: 'tokens',
      label: 'Links & Tokens',
      count: String(store.ownerTokens.length),
      icon: KeyRound
    },
    {
      id: 'trash',
      label: 'Trash',
      count: store.ownerTrash.length
        ? formatBytes(store.ownerTrash.reduce((a, b) => a + b.size, 0))
        : '0 B',
      icon: Trash2
    }
  ]);
</script>

<aside class="sidebar" class:collapsed class:mobile>
  <div class="brand">
    <div class="brand-lockup">
      <img src="/branding/icon.svg" alt="Flora" class="brand-icon" />
      {#if !collapsed}
        <span class="brand-word">Flora</span>
      {/if}
    </div>

    {#if mobile}
      <button
        class="btn-icon"
        onclick={() => (store.mobileNavOpen = false)}
        aria-label="Close navigation"
      >
        <X size={16} />
      </button>
    {:else if !collapsed}
      <div class="brand-right">
        <span class="server-badge">v{store.server.version}</span>
        <button
          class="btn-icon"
          title="Collapse sidebar"
          onclick={() => (store.leftCollapsed = true)}
        >
          <PanelLeftClose size={15} />
        </button>
      </div>
    {/if}
  </div>

  {#if collapsed}
    <button
      class="btn-icon expand-rail-btn"
      title="Expand sidebar"
      onclick={() => (store.leftCollapsed = false)}
    >
      <PanelLeftOpen size={15} />
    </button>
  {/if}

  {#if !collapsed}
    <div class="space-wrap">
      <button
        class="space-switcher"
        onclick={() => (spaceMenuOpen = !spaceMenuOpen)}
        title="Switch storage space"
      >
        <div class="space-meta">
          <span class="space-label">
            <span
              class="role-dot"
              class:guest={store.selectedOwnerId !== store.authUser?.id}
            ></span>
            <span>{user.name} · {user.admin ? 'admin' : 'user'}</span>
          </span>
          <span class="space-uuid">{user.id.slice(0, 18)}…</span>
        </div>
        <ChevronsUpDown size={14} class="dim-icon" />
      </button>

      {#if spaceMenuOpen}
        <div class="space-menu">
          {#each store.users as u (u.id)}
            <button
              class="space-option"
              class:active={u.id === store.selectedOwnerId}
              onclick={() => {
                void store.switchSpace(u.id);
                spaceMenuOpen = false;
              }}
            >
              <div>
                <div class="opt-name">{u.name}</div>
                <div class="opt-id mono">{u.id.slice(0, 13)}…</div>
              </div>
              <span class="badge" class:shared={u.admin}>{u.admin ? 'admin' : 'user'}</span>
            </button>
          {/each}
          <div class="space-menu-divider"></div>
          <button
            class="space-option signout-option"
            onclick={() => {
              spaceMenuOpen = false;
              void store.signOut();
            }}
          >
            <span class="signout-label">
              <LogOut size={13} />
              <span>Sign out</span>
            </span>
          </button>
        </div>
      {/if}
    </div>
  {/if}

  <nav class="nav-section" aria-label="Workspace">
    {#if !collapsed}
      <div class="nav-heading">Workspace</div>
    {/if}
    {#each navItems as item (item.id)}
      {@const Icon = item.icon}
      <button
        class="nav-btn"
        class:active={store.tab === item.id}
        title={collapsed ? `${item.label} (${item.count})` : undefined}
        onclick={() => store.selectTab(item.id)}
      >
        <span class="nav-btn-left">
          <Icon size={16} strokeWidth={1.8} class="nav-icon" />
          {#if !collapsed}
            <span>{item.label}</span>
          {/if}
        </span>
        {#if !collapsed}
          <span class="nav-count">{item.count}</span>
        {/if}
      </button>
    {/each}
  </nav>

  {#if store.isAdmin}
    <nav class="nav-section" aria-label="Administration">
      {#if !collapsed}
        <div class="nav-heading">Administration</div>
      {/if}
      <button
        class="nav-btn"
        class:active={store.tab === 'admin'}
        title={collapsed ? 'Server & Users' : undefined}
        onclick={() => store.selectTab('admin')}
      >
        <span class="nav-btn-left">
          <Server size={16} strokeWidth={1.8} class="nav-icon" />
          {#if !collapsed}
            <span>Server &amp; Users</span>
          {/if}
        </span>
        {#if !collapsed}
          <span class="nav-count live">{store.users.length}</span>
        {/if}
      </button>
    </nav>
  {/if}

  <div class="sidebar-spacer"></div>

  {#if !collapsed}
    <div class="meters-card">
      <div class="meter-block">
        <div class="meter-row">
          <span class="meter-title">Quota</span>
          <span class="meter-val">
            {formatBytes(user.usedBytes + user.trashBytes)} / {quotaBytes > 0 ? formatBytes(quotaBytes) : '∞'}
          </span>
        </div>
        <div class="meter-track">
          <div class="meter-fill-live" style:width="{liveQuotaPct}%"></div>
          <div class="meter-fill-trash" style:width="{trashQuotaPct}%"></div>
        </div>
        <div class="meter-legend">
          <span class="legend-item">
            <span class="legend-dot" style="background:var(--accent)"></span>
            {formatBytes(user.usedBytes)} live
          </span>
          <span class="legend-item">
            <span class="legend-dot" style="background:var(--warn)"></span>
            {formatBytes(user.trashBytes)} trash
          </span>
        </div>
      </div>

      <div class="meter-block divider">
        <div class="meter-row">
          <span class="meter-title">Upload buffer</span>
          <span class="meter-val">{formatBytes(bufferBytes)} / {formatBytes(bufferSize)}</span>
        </div>
        <div class="meter-track">
          <div class="meter-fill-buffer" style:width="{bufferPct}%"></div>
        </div>
      </div>
    </div>
  {/if}
</aside>

<style>
  .sidebar {
    container-type: inline-size;
    background: var(--surface-0);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 16px 13px 14px;
    gap: 16px;
    height: 100%;
    overflow-y: auto;
  }

  .sidebar.collapsed {
    padding: 16px 10px 14px;
    align-items: center;
    gap: 14px;
  }

  .sidebar.mobile {
    width: 100%;
    border-right: none;
  }

  .brand {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .brand-lockup {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .brand-icon {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    flex-shrink: 0;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 23px;
    color: var(--head);
    transform: translateY(1px);
  }

  .brand-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .server-badge {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 7px;
  }

  .expand-rail-btn {
    width: 34px;
    height: 34px;
  }

  .space-wrap {
    position: relative;
  }

  .space-switcher {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 8px 10px;
    text-align: left;
    transition: border-color 120ms;
  }

  .space-switcher:hover {
    border-color: var(--border-hover);
  }

  .space-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .space-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--head);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .role-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }

  .role-dot.guest {
    background: var(--warn);
  }

  .space-uuid {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  :global(.dim-icon) {
    color: var(--muted);
    flex-shrink: 0;
  }

  .space-menu {
    position: absolute;
    top: calc(100% + 5px);
    left: 0;
    right: 0;
    background: var(--surface-2);
    border: 1px solid var(--border-hover);
    border-radius: var(--r-sm);
    box-shadow: 0 14px 34px rgba(0, 0, 0, 0.58);
    padding: 4px;
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .space-option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 5px;
    text-align: left;
    color: var(--fg);
    font-size: 12.5px;
  }

  .space-option:hover {
    background: var(--surface-3);
  }

  .space-option.active {
    background: var(--accent-soft);
  }

  .opt-name {
    color: var(--head);
    font-weight: 500;
  }

  .opt-id {
    font-size: 10px;
    color: var(--muted);
  }

  .space-menu-divider {
    height: 1px;
    background: var(--border);
    margin: 3px 4px;
  }

  .signout-label {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    color: var(--dim);
  }

  .nav-section {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav-heading {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.09em;
    color: var(--muted);
    font-weight: 600;
    padding: 0 8px 5px;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 7px 8px;
    border-radius: var(--r-sm);
    color: var(--dim);
    font-size: 12.5px;
    transition: background 100ms, color 100ms;
  }

  .nav-btn:hover {
    background: var(--surface-1);
    color: var(--fg);
  }

  .nav-btn.active {
    background: var(--surface-2);
    color: var(--head);
  }

  .nav-btn-left {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  :global(.nav-icon) {
    flex-shrink: 0;
  }

  .nav-count {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--muted);
    flex-shrink: 0;
  }

  .nav-count.live {
    color: var(--accent);
  }

  .sidebar-spacer {
    flex: 1;
    min-height: 12px;
  }

  .meters-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 11px 11px 12px;
    display: flex;
    flex-direction: column;
    gap: 11px;
  }

  .meter-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .meter-block.divider {
    border-top: 1px solid var(--border-subtle);
    padding-top: 10px;
  }

  .meter-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }

  .meter-title {
    font-size: 11px;
    color: var(--dim);
  }

  .meter-val {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--muted);
  }

  .meter-track {
    height: 5px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
    display: flex;
  }

  .meter-fill-live {
    background: var(--accent);
  }

  .meter-fill-trash {
    background: var(--warn);
  }

  .meter-fill-buffer {
    background: #8a9c7a;
  }

  .meter-legend {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 10px;
    color: var(--muted);
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .legend-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
</style>
