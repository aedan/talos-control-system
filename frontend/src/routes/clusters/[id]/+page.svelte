<script lang="ts">
  import { page } from '$app/stores';
  import { client } from '$lib/api/client';
  import { onMount } from 'svelte';
  
  let cluster = $state(null as any);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      cluster = await client.get(`/clusters/${$page.params.id}`);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load cluster';
    } finally {
      loading = false;
    }
  });
</script>

<div class="cluster-detail">
{#if loading}
    <p>Loading...</p>
{:else if error}
    <div class="error-banner">{error}</div>
{:else if cluster}
    <h1>{cluster.name}</h1>
    
    <div class="info-grid">
      <div class="info-item">
        <span class="info-label">Kubernetes</span>
        <span class="info-value">{cluster.controlPlaneVersion}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Talos</span>
        <span class="info-value">{cluster.talosVersion}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Status</span>
        <span class="info-value">{cluster.status}</span>
      </div>
    </div>
    
    <nav class="tabs">
      <a href="/clusters/{$page.params.id}/nodes">Nodes</a>
      <a href="/clusters/{$page.params.id}/machines">Machines</a>
      <a href="/clusters/{$page.params.id}/config">Config</a>
      <a href="/clusters/{$page.params.id}/backups">Backups</a>
    </nav>
    
    <slot />
  {/if}
</div>

<style>
  .cluster-detail h1 { margin: 0 0 1.5rem; }
  .info-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1rem; margin-bottom: 2rem; }
  .info-item { display: flex; flex-direction: column; gap: 0.25rem; }
  .info-label { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .info-value { font-size: 1.1rem; font-weight: 500; }
  
  .tabs { display: flex; gap: 0; border-bottom: 1px solid var(--tcs-border); margin-bottom: 1.5rem; }
  .tabs a {
    padding: 0.75rem 1rem;
    color: var(--tcs-text-muted);
    border-bottom: 2px solid transparent;
    transition: all 0.15s;
  }
  .tabs a:hover { color: var(--tcs-text); text-decoration: none; }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem 1rem;
    color: var(--tcs-error, #ef4444);
    font-size: 0.875rem;
    margin-bottom: 1.5rem;
  }
</style>
