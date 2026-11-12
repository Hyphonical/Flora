<script lang="ts">
  import { ArrowRight } from '@lucide/svelte';
  import { store } from '../lib/store.svelte';

  let username = $state('');
  let password = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = null;
    try {
      await store.submitLogin(username, password);
    } catch (err) {
      error = err instanceof Error ? err.message : 'Sign in failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="standalone-stage">
  <form class="auth-card" onsubmit={handleSubmit}>
    <div class="auth-brand">
      <img src="/branding/icon.svg" alt="Flora" class="setup-logo" />
      <span class="brand-word">Flora</span>
    </div>

    <div>
      <h1 class="editorial-title">Sign in</h1>
      <p class="auth-sub">Enter your account credentials to open your storage space.</p>
    </div>

    {#if error}
      <p class="auth-error">{error}</p>
    {/if}

    <label class="form-field">
      <span>Username</span>
      <input
        class="input"
        type="text"
        required
        autocomplete="username"
        value={username}
        oninput={(e) => (username = e.currentTarget.value)}
      />
    </label>

    <label class="form-field">
      <span>Password</span>
      <input
        class="input"
        type="password"
        required
        autocomplete="current-password"
        value={password}
        oninput={(e) => (password = e.currentTarget.value)}
      />
    </label>

    <button type="submit" class="btn btn-primary submit-btn" disabled={busy}>
      <span>{busy ? 'Signing in…' : 'Sign in'}</span>
      <ArrowRight size={14} />
    </button>
  </form>
</div>

<style>
  .standalone-stage {
    min-height: 100%;
    display: grid;
    place-items: center;
    padding: 24px 16px;
    background: radial-gradient(circle at 50% 0%, #1f211b 0%, var(--bg) 52%);
  }

  .auth-card {
    width: 100%;
    max-width: 400px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    padding: 26px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .auth-brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .setup-logo {
    width: 30px;
    height: 30px;
    border-radius: 7px;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 25px;
    color: var(--head);
    transform: translateY(1px);
  }

  .auth-sub {
    color: var(--dim);
    font-size: 12.5px;
    margin-top: 6px;
  }

  .auth-error {
    color: #e69d79;
    background: var(--danger-soft);
    border: 1px solid rgba(192, 109, 68, 0.4);
    border-radius: var(--r-sm);
    padding: 8px 10px;
    font-size: 12px;
  }

  .form-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-field span {
    font-size: 12px;
    color: var(--dim);
  }

  .submit-btn {
    padding: 9px 14px;
    margin-top: 4px;
    justify-content: center;
  }
</style>
