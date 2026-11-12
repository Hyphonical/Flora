<script lang="ts">
  import { UserPlus, Trash2, ArrowUpRight, RefreshCw } from '@lucide/svelte';
  import { store, formatBytes, formatTimestamp } from '../lib/store.svelte';

  let showCreateUser = $state(false);
  let newUsername = $state('');
  let newPassword = $state('');
  let newQuotaGb = $state('20');
  let newAdmin = $state(false);
  let busy = $state(false);

  const totalUsed = $derived(
    store.dashboardStats.reduce((sum, u) => sum + u.used, 0)
  );
  const totalBuffer = $derived(
    store.dashboardStats.reduce((sum, u) => sum + u.buffer_reserved, 0)
  );
  const bufferSize = $derived(store.dashboardBufferSize || store.server.buffer_size);

  async function addUser() {
    const clean = newUsername.trim();
    if (!clean || busy) return;
    busy = true;
    const quotaGb = Number(newQuotaGb);
    const created = await store.createUserAccount({
      name: clean,
      password: newPassword,
      admin: newAdmin,
      quotaBytes: quotaGb > 0 ? Math.round(quotaGb * 1_000_000_000) : null
    });
    busy = false;
    if (created) {
      newUsername = '';
      newPassword = '';
      newAdmin = false;
      showCreateUser = false;
    }
  }
</script>

<div>
  <div class="page-header">
    <div>
      <h1 class="editorial-title">Server &amp; Users</h1>
      <div class="page-meta-line">
        <span>{store.users.length} accounts</span>
        <span>·</span>
        <span class="mono">{formatBytes(totalUsed)} stored</span>
      </div>
    </div>

    <div class="header-controls">
      <button
        class="btn"
        onclick={() => {
          void Promise.all([
            store.refreshDashboard(),
            store.refreshUsers(),
            store.refreshActivity(),
            store.refreshTransfers()
          ]);
          store.notify('Refreshed server data');
        }}
      >
        <RefreshCw size={14} />
        <span>Refresh</span>
      </button>
      <button class="btn btn-danger" onclick={() => store.purgeAllAdmin()}>
        <Trash2 size={14} />
        <span>Purge all</span>
      </button>
      <button
        class="btn btn-primary"
        onclick={() => (showCreateUser = !showCreateUser)}
      >
        <UserPlus size={14} />
        <span>New user</span>
      </button>
    </div>
  </div>

  {#if showCreateUser}
    <div class="create-user-bar">
      <input
        class="input"
        placeholder="Username"
        value={newUsername}
        oninput={(e) => (newUsername = e.currentTarget.value)}
      />
      <input
        class="input"
        type="password"
        placeholder="Password"
        autocomplete="new-password"
        value={newPassword}
        oninput={(e) => (newPassword = e.currentTarget.value)}
      />
      <div class="quota-field">
        <span>Quota (GB)</span>
        <input
          class="input mono"
          type="number"
          min="0"
          style="width:86px;"
          value={newQuotaGb}
          oninput={(e) => (newQuotaGb = e.currentTarget.value)}
        />
      </div>
      <label class="admin-check">
        <input
          type="checkbox"
          checked={newAdmin}
          onchange={(e) => (newAdmin = e.currentTarget.checked)}
        />
        <span>admin</span>
      </label>
      <button
        class="btn btn-primary btn-sm"
        disabled={busy || !newUsername.trim() || newPassword.length < 8}
        onclick={addUser}
      >
        Create
      </button>
      <button class="btn btn-sm" onclick={() => (showCreateUser = false)}>Cancel</button>
    </div>
  {/if}

  <div class="cards-row">
    <div class="stat-card">
      <div class="stat-label">Storage used</div>
      <div class="stat-value">{formatBytes(totalUsed)}</div>
      <div class="stat-note mono">{store.users.length} accounts</div>
    </div>

    <div class="stat-card">
      <div class="stat-label">Upload buffer</div>
      <div class="stat-value">{formatBytes(totalBuffer)}</div>
      <div class="stat-note mono">
        {formatBytes(bufferSize)} ceiling · {store.activeTransfers.length} active
      </div>
    </div>

    <div class="stat-card">
      <div class="stat-label">Live transfers</div>
      <div class="stat-value">{store.activeTransfers.length}</div>
      <div class="stat-note mono">
        {store.transfers.length - store.activeTransfers.length} completed
      </div>
    </div>
  </div>

  <section class="admin-section">
    <div class="section-head">
      <span>Accounts</span>
    </div>

    <div class="ledger">
      <div class="ledger-head user-cols">
        <span>User</span>
        <span style="text-align:right;">Usage</span>
        <span class="col-buf">Buffer</span>
        <span class="col-ret">Retention</span>
        <span style="text-align:right;">Space</span>
      </div>

      {#each store.users as u (u.id)}
        <div class="ledger-row user-cols">
          <div class="user-cell">
            <span class="user-name">{u.name}</span>
            <span class="badge" class:shared={u.admin}>{u.admin ? 'admin' : 'user'}</span>
            <span class="mono uuid-sub">{u.id.slice(0, 13)}…</span>
          </div>

          <div class="mono right-cell">
            {formatBytes(u.usedBytes + u.trashBytes)} / {u.quotaBytes ? formatBytes(u.quotaBytes) : '∞'}
          </div>

          <div class="mono dim-cell col-buf">{formatBytes(u.bufferReservedBytes)}</div>

          <div class="mono dim-cell col-ret">{u.retention}</div>

          <div class="act-cell">
            <button
              class="btn btn-sm"
              onclick={() => {
                void store.switchSpace(u.id);
                store.selectTab('files');
              }}
            >
              <span>Open</span>
              <ArrowUpRight size={12} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section class="admin-section">
    <div class="section-head">
      <span>Recent activity</span>
    </div>

    <div class="ledger">
      <div class="ledger-head act-cols">
        <span>Request</span>
        <span style="text-align:right;">Status</span>
        <span class="col-ip">Client IP</span>
        <span style="text-align:right;">Time</span>
      </div>

      {#each store.activity as ev, idx (`${ev.time}-${idx}`)}
        <div class="ledger-row act-cols">
          <div class="req-cell">
            <span
              class="badge"
              class:shared={ev.method === 'POST' && ev.status < 400}
              class:warn={ev.method === 'DELETE'}
              class:danger={ev.status >= 400}
            >
              {ev.method}
            </span>
            <span class="mono req-path">{ev.path}</span>
          </div>

          <div
            class="mono right-cell"
            style:color={ev.status >= 400 ? 'var(--danger)' : 'var(--accent)'}
          >
            {ev.status}
          </div>

          <div class="mono dim-cell col-ip">{ev.client}</div>
          <div class="mono right-cell dim-cell">{formatTimestamp(ev.time)}</div>
        </div>
      {:else}
        <div class="ledger-row empty-row">No activity recorded yet.</div>
      {/each}
    </div>
  </section>
</div>

<style>
  .create-user-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 16px;
    padding: 10px 12px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    flex-wrap: wrap;
  }

  .quota-field {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
  }

  .admin-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
  }

  .cards-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 12px;
    margin-bottom: 20px;
  }

  .stat-card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 14px 16px;
  }

  .stat-label {
    font-size: 11.5px;
    color: var(--dim);
  }

  .stat-value {
    font-family: var(--font-display);
    font-size: 30px;
    color: var(--head);
    line-height: 1.1;
    margin-top: 4px;
  }

  .stat-note {
    font-size: 11px;
    color: var(--dim);
    margin-top: 6px;
  }

  .admin-section {
    margin-bottom: 20px;
  }

  .section-head {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--dim);
    font-weight: 600;
    margin-bottom: 8px;
  }

  .user-cols {
    display: grid;
    grid-template-columns: minmax(200px, 1.4fr) 150px 95px 86px 90px;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
  }

  .act-cols {
    display: grid;
    grid-template-columns: minmax(200px, 1.5fr) 68px 130px 110px;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
  }

  .user-cell,
  .req-cell {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .user-name {
    font-weight: 500;
    color: var(--head);
  }

  .uuid-sub {
    font-size: 11px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .right-cell {
    text-align: right;
    color: var(--fg);
    font-size: 12px;
  }

  .dim-cell {
    color: var(--dim);
    font-size: 12px;
  }

  .req-path {
    font-size: 11.5px;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .act-cell {
    display: flex;
    justify-content: flex-end;
  }

  .empty-row {
    padding: 22px 14px;
    color: var(--dim);
    font-size: 12.5px;
  }

  @media (max-width: 820px) {
    .user-cols,
    .act-cols {
      grid-template-columns: minmax(150px, 1fr) 96px;
    }

    .col-buf,
    .col-ret,
    .col-ip {
      display: none;
    }
  }
</style>
