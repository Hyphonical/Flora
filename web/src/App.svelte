<script lang="ts">
  import { store } from './lib/store.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import Topbar from './components/Topbar.svelte';
  import Inspector from './components/Inspector.svelte';
  import UploadDock from './components/UploadDock.svelte';
  import FilesView from './components/FilesView.svelte';
  import PastesView from './components/PastesView.svelte';
  import TokensView from './components/TokensView.svelte';
  import TrashView from './components/TrashView.svelte';
  import AdminView from './components/AdminView.svelte';
  import PublicShareView from './components/PublicShareView.svelte';
  import SetupView from './components/SetupView.svelte';
  import LoginView from './components/LoginView.svelte';

  const LEFT_MIN = 188;
  const LEFT_MAX = 340;
  const LEFT_RAIL = 60;

  const RIGHT_MIN = 264;
  const RIGHT_MAX = 460;

  let draggingSide = $state<'left' | 'right' | null>(null);

  const showRightDesktop = $derived(store.tab === 'files' && !store.rightCollapsed);

  const effectiveLeftPx = $derived(store.leftCollapsed ? LEFT_RAIL : store.leftWidth);

  function startResize(side: 'left' | 'right', e: PointerEvent) {
    e.preventDefault();
    draggingSide = side;
  }

  function handlePointerMove(e: PointerEvent) {
    if (!draggingSide) return;
    if (draggingSide === 'left') {
      if (e.clientX < 105) {
        store.leftCollapsed = true;
      } else {
        store.leftCollapsed = false;
        store.leftWidth = Math.max(LEFT_MIN, Math.min(LEFT_MAX, e.clientX));
      }
    } else if (draggingSide === 'right') {
      const fromRight = window.innerWidth - e.clientX;
      if (fromRight < 130) {
        store.rightCollapsed = true;
      } else {
        store.rightCollapsed = false;
        store.rightWidth = Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, fromRight));
      }
    }
  }

  function stopResize() {
    draggingSide = null;
  }
</script>

<svelte:window onpointermove={handlePointerMove} onpointerup={stopResize} />

<div class="root-frame" class:is-resizing={draggingSide !== null}>
  <div class="stage-body">
    {#if store.scene === 'loading'}
      <div class="boot-stage">
        <img src="/branding/icon.svg" alt="Flora" class="boot-logo" />
        <span class="boot-label">Loading Flora…</span>
      </div>
    {:else if store.scene === 'workspace'}
      <div
        class="app-shell"
        class:has-inspector={showRightDesktop}
        style:--left-col="{effectiveLeftPx}px"
        style:--right-col="{store.rightWidth}px"
      >
        <div class="desktop-sidebar-slot">
          <Sidebar />
          <div
            class="resize-handle left-handle"
            class:active={draggingSide === 'left'}
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize navigation sidebar"
            onpointerdown={(e) => startResize('left', e)}
            ondblclick={() => {
              store.leftCollapsed = false;
              store.leftWidth = 240;
            }}
          ></div>
        </div>

        <main class="workspace">
          <Topbar />
          <div class="content-scroll">
            {#if store.tab === 'files'}
              <FilesView />
            {:else if store.tab === 'pastes'}
              <PastesView />
            {:else if store.tab === 'tokens'}
              <TokensView />
            {:else if store.tab === 'trash'}
              <TrashView />
            {:else if store.tab === 'admin'}
              <AdminView />
            {/if}
          </div>
        </main>

        {#if showRightDesktop}
          <div class="desktop-inspector-slot">
            <div
              class="resize-handle right-handle"
              class:active={draggingSide === 'right'}
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize details panel"
              onpointerdown={(e) => startResize('right', e)}
              ondblclick={() => (store.rightWidth = 332)}
            ></div>
            <Inspector />
          </div>
        {/if}
      </div>

      <!-- Off-canvas Mobile Navigation Drawer -->
      {#if store.mobileNavOpen}
        <div
          class="sheet-backdrop"
          role="presentation"
          onclick={() => (store.mobileNavOpen = false)}
        ></div>
        <div class="mobile-drawer left-drawer">
          <Sidebar mobile={true} />
        </div>
      {/if}

      <!-- Slide-over Tablet/Mobile Inspector Drawer -->
      {#if store.mobileInspectorOpen}
        <div
          class="sheet-backdrop"
          role="presentation"
          onclick={() => (store.mobileInspectorOpen = false)}
        ></div>
        <div class="mobile-drawer right-drawer">
          <Inspector mobile={true} />
        </div>
      {/if}

      <UploadDock />
    {:else if store.scene === 'share'}
      <PublicShareView />
    {:else if store.scene === 'login'}
      <LoginView />
    {:else if store.scene === 'setup'}
      <SetupView />
    {/if}
  </div>

  {#if store.toastMessage}
    <div class="toast" role="status">{store.toastMessage}</div>
  {/if}
</div>

<style>
  .root-frame {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .root-frame.is-resizing {
    cursor: col-resize;
    user-select: none;
  }

  .stage-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    position: relative;
  }

  .boot-stage {
    min-height: 100%;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    color: var(--dim);
    font-size: 12.5px;
  }

  .boot-logo {
    width: 34px;
    height: 34px;
    border-radius: 8px;
  }

  .app-shell {
    display: grid;
    grid-template-columns: var(--left-col) minmax(0, 1fr);
    height: 100%;
    overflow: hidden;
  }

  .app-shell.has-inspector {
    grid-template-columns: var(--left-col) minmax(0, 1fr) var(--right-col);
  }

  .desktop-sidebar-slot,
  .desktop-inspector-slot {
    position: relative;
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }

  .resize-handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 6px;
    z-index: 15;
    cursor: col-resize;
    transition: background 120ms;
  }

  .left-handle {
    right: 0;
  }

  .right-handle {
    left: 0;
  }

  .resize-handle:hover,
  .resize-handle.active {
    background: rgba(175, 185, 122, 0.45);
  }

  .workspace {
    display: flex;
    flex-direction: column;
    min-width: 0;
    height: 100%;
    background: var(--bg);
    overflow: hidden;
  }

  .content-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 22px 26px 96px;
  }

  /* Mobile & Tablet Drawers */
  .sheet-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(2px);
    z-index: 45;
  }

  .mobile-drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    z-index: 50;
    width: min(320px, 88vw);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65);
  }

  .left-drawer {
    left: 0;
    width: min(272px, 84vw);
  }

  .right-drawer {
    right: 0;
    width: min(350px, 92vw);
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    background: var(--surface-3);
    border: 1px solid var(--border-hover);
    color: var(--head);
    padding: 7px 14px;
    border-radius: 999px;
    font-size: 12.5px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.55);
    z-index: 60;
  }

  @media (max-width: 1024px) {
    .app-shell,
    .app-shell.has-inspector {
      grid-template-columns: var(--left-col) minmax(0, 1fr);
    }

    .desktop-inspector-slot {
      display: none;
    }
  }

  @media (max-width: 768px) {
    .app-shell,
    .app-shell.has-inspector {
      grid-template-columns: minmax(0, 1fr);
    }

    .desktop-sidebar-slot {
      display: none;
    }

    .content-scroll {
      padding: 16px 14px 110px;
    }
  }
</style>
