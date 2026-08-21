<script lang="ts">
  import { branding, applyBranding } from '$lib/stores/branding';
  import Logo from '$lib/branding/components/Logo.svelte';
  import { afterNavigate, goto } from '$app/navigation';
  import { onMount, onDestroy } from 'svelte';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();

  let authenticated = $state(false);
  let checking = $state(true);
  let user = $state<any>(null);
  let settingsOpen = $state(false);
  let settingsMenu = $state<HTMLElement | null>(null);
  let authGen = 0;

  function onDocClick(e: MouseEvent) {
    if (settingsOpen && settingsMenu && !settingsMenu.contains(e.target as Node)) {
      settingsOpen = false;
    }
  }

  async function checkAuth() {
    if (typeof window === 'undefined') {
      checking = false;
      return;
    }

    const gen = ++authGen;
    checking = true;

    // Login route: no shell chrome; do not require a token.
    if (window.location.pathname === '/login') {
      authenticated = false;
      user = null;
      if (gen === authGen) checking = false;
      return;
    }

    const token = localStorage.getItem('tcs_token');
    if (!token) {
      authenticated = false;
      user = null;
      if (gen === authGen) checking = false;
      goto('/login');
      return;
    }

    try {
      const res = await fetch('/api/auth/me', {
        headers: { Authorization: `Bearer ${token}` }
      });
      if (gen !== authGen) return;
      if (!res.ok) {
        localStorage.removeItem('tcs_token');
        authenticated = false;
        user = null;
        checking = false;
        goto('/login');
        return;
      }
      user = await res.json();
      authenticated = true;
    } catch {
      if (gen !== authGen) return;
      localStorage.removeItem('tcs_token');
      authenticated = false;
      user = null;
      goto('/login');
    } finally {
      if (gen === authGen) checking = false;
    }
  }

  onMount(async () => {
    const m = await import('$lib/stores/branding');
    m.fetchBranding();
    document.addEventListener('click', onDocClick);
  });

  onDestroy(() => {
    document.removeEventListener('click', onDocClick);
  });

  // Re-run after every navigation (including login → /). Root layout stays
  // mounted across client-side goto(), so onMount alone never re-auths.
  afterNavigate(() => {
    settingsOpen = false;
    void checkAuth();
  });

  $effect(() => {
    applyBranding($branding);
  });

  async function handleLogout() {
    localStorage.removeItem('tcs_token');
    authenticated = false;
    user = null;
    goto('/login');
  }
</script>

{#if checking}
<!-- Loading overlay while auth resolves -->
<div class="loading-overlay">
  <div class="spinner"></div>
</div>
{:else if authenticated}
<div class="layout">
  <header class="topbar">
    <a href="/" class="topbar-brand">
      <Logo size="sm" />
    </a>
    <div class="topbar-right">
      <div class="settings-menu" bind:this={settingsMenu}>
        <button
          type="button"
          class="settings-toggle"
          class:open={settingsOpen}
          onclick={() => (settingsOpen = !settingsOpen)}
          title="Open settings: certificates, auth, branding, siderolink, metal/PXE, users, audit logs, system"
        >
          Settings
          <span class="caret">▾</span>
        </button>
        {#if settingsOpen}
          <ul class="settings-dropdown">
            <li><a href="/settings/certificates" title="Manage the TCS HTTPS certificate (self-signed, provided, or Let's Encrypt)">Certificates</a></li>
            <li><a href="/settings/auth" title="Configure local, LDAP, OIDC, and SAML authentication">Auth</a></li>
            <li><a href="/settings/branding" title="Set the product name, logo, and theme colors">Branding</a></li>
            <li><a href="/settings/siderolink" title="Manage the SideroLink WireGuard mesh for out-of-band node access">Siderolink</a></li>
            <li><a href="/settings/metal" title="Configure the metal provisioning server (DHCP/PXE) for bare-metal installs">Metal / PXE</a></li>
            <li><a href="/settings/users" title="Create, edit, and disable TCS users and roles">Users</a></li>
            <li><a href="/settings/audit-logs" title="Review the immutable audit trail of TCS actions">Audit Logs</a></li>
            <li><a href="/settings/system" title="View TCS version, uptime, database, and runtime info">System</a></li>
          </ul>
        {/if}
      </div>
      {#if user}
        <span class="user-info">{user.displayName || user.display_name || user.email}</span>
      {/if}
      <button type="button" class="logout-btn" title="Sign out of TCS on this device" onclick={handleLogout}>Logout</button>
    </div>
  </header>
  <div class="content">
    {@render children()}
  </div>
</div>
{:else}
<!-- Login page or redirecting — render children -->
{@render children()}
{/if}

<style>
  .loading-overlay {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    background: var(--tcs-background);
  }

  .spinner {
    width: 32px;
    height: 32px;
    border: 3px solid var(--tcs-border);
    border-top-color: var(--tcs-primary);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .layout {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
  }

  .topbar {
    height: 56px;
    padding: 0 1.5rem;
    border-bottom: 1px solid var(--tcs-border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--tcs-surface);
    flex-shrink: 0;
  }

  .topbar-brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    text-decoration: none;
  }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .settings-menu {
    position: relative;
  }

  .settings-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.4rem 0.7rem;
    color: var(--tcs-text);
    cursor: pointer;
    font: inherit;
    font-size: 0.875rem;
    transition: all 0.15s ease;
  }

  .settings-toggle:hover,
  .settings-toggle.open {
    background: var(--tcs-surface-hover);
  }

  .caret {
    font-size: 0.7rem;
  }

  .settings-dropdown {
    position: absolute;
    top: calc(100% + 0.4rem);
    right: 0;
    min-width: 180px;
    margin: 0;
    padding: 0.35rem;
    list-style: none;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    z-index: 50;
  }

  .settings-dropdown li a {
    display: block;
    padding: 0.5rem 0.75rem;
    border-radius: 6px;
    color: var(--tcs-text-muted);
    text-decoration: none;
    font-size: 0.875rem;
    transition: all 0.15s ease;
  }

  .settings-dropdown li a:hover {
    background: var(--tcs-surface-hover);
    color: var(--tcs-text);
  }

  .user-info {
    color: var(--tcs-text-muted);
    font-size: 0.85rem;
  }

  .logout-btn {
    background: none;
    border: none;
    padding: 0.4rem 0.2rem;
    color: var(--tcs-text-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.875rem;
    transition: all 0.15s ease;
  }

  .logout-btn:hover {
    color: var(--tcs-text);
  }

  .content {
    flex: 1;
    padding: 1.5rem;
  }
</style>
