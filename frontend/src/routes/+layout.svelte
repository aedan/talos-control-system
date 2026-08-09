<script lang="ts">
  import { branding, applyBranding } from '$lib/stores/branding';
  import Logo from '$lib/branding/components/Logo.svelte';
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();

  let authenticated = $state(false);
  let checking = $state(true);
  let user = $state<any>(null);

  async function checkAuth() {
    if (typeof window === 'undefined') {
      checking = false;
      return;
    }

    // Never redirect on /login
    if (window.location.pathname === '/login') {
      checking = false;
      return;
    }

    const token = localStorage.getItem('tcs_token');
    if (!token) {
      goto('/login');
      return;
    }

    try {
      const res = await fetch('/api/auth/me', {
        headers: { 'Authorization': `Bearer ${token}` }
      });
      if (!res.ok) {
        localStorage.removeItem('tcs_token');
        goto('/login');
        return;
      }
      user = await res.json();
      authenticated = true;
    } catch {
      localStorage.removeItem('tcs_token');
      goto('/login');
    } finally {
      checking = false;
    }
  }

  onMount(async () => {
    const m = await import('$lib/stores/branding');
    m.fetchBranding();
    await checkAuth();
  });

  $effect(() => {
    applyBranding($branding);
  });

  async function handleLogout() {
    localStorage.removeItem('tcs_token');
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
  <nav class="sidebar">
    <a href="/" class="sidebar-logo">
      <Logo size="sm" />
    </a>
    <ul class="sidebar-nav">
      <li><a href="/clusters">Clusters</a></li>
      <li><a href="/machines">Machines</a></li>
      <li><a href="/machine-classes">Machine Classes</a></li>
    </ul>
    <ul class="sidebar-nav sidebar-nav-bottom">
      <li><a href="/settings">Settings</a></li>
      <li class="sub"><a href="/settings/certificates">Certificates</a></li>
      <li class="sub"><a href="/settings/auth">Auth</a></li>
      <li class="sub"><a href="/settings/branding">Branding</a></li>
      <li class="sub"><a href="/settings/users">Users</a></li>
      <li><button type="button" class="logout-btn" onclick={handleLogout}>Logout</button></li>
    </ul>
  </nav>

  <main class="main">
    <header class="topbar">
      <span class="brand">{$branding.shortName}</span>
      {#if user}
        <span class="user-info">{user.displayName || user.display_name || user.email}</span>
      {/if}
    </header>
    <div class="content">
      {@render children()}
    </div>
  </main>
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
    min-height: 100vh;
  }

  .sidebar {
    width: 240px;
    background: var(--tcs-surface);
    border-right: 1px solid var(--tcs-border);
    display: flex;
    flex-direction: column;
    padding: 1rem 0;
    flex-shrink: 0;
  }

  .sidebar-logo {
    padding: 0 1rem 1.5rem;
    border-bottom: 1px solid var(--tcs-border);
    margin-bottom: 0.5rem;
  }

  .sidebar-nav {
    list-style: none;
    padding: 0;
    margin: 0;
    flex: 1;
  }

  .sidebar-nav li a {
    display: block;
    padding: 0.6rem 1.2rem;
    color: var(--tcs-text-muted);
    transition: all 0.15s ease;
  }

  .sidebar-nav li a:hover {
    background: var(--tcs-surface-hover);
    color: var(--tcs-text);
    text-decoration: none;
  }

  .sidebar-nav li.sub a {
    padding-left: 2rem;
    font-size: 0.9em;
  }

  .logout-btn {
    background: none;
    border: none;
    padding: 0.6rem 1.2rem;
    color: var(--tcs-text-muted);
    cursor: pointer;
    font: inherit;
    text-align: left;
    width: 100%;
    transition: all 0.15s ease;
  }

  .logout-btn:hover {
    background: var(--tcs-surface-hover);
    color: var(--tcs-text);
  }

  .sidebar-nav-bottom {
    border-top: 1px solid var(--tcs-border);
    margin-top: auto;
    padding-top: 0.5rem;
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .topbar {
    height: 56px;
    padding: 0 1.5rem;
    border-bottom: 1px solid var(--tcs-border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .brand {
    font-size: 1.1rem;
    font-weight: 600;
  }

  .user-info {
    color: var(--tcs-text-muted);
    font-size: 0.85rem;
  }

  .content {
    flex: 1;
    padding: 1.5rem;
  }
</style>
