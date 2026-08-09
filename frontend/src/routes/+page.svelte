<script lang="ts">
  import { onMount } from 'svelte';
  import { clusters, loading, error, clusterCount, loadClusters } from '$lib/stores/clusters';
  import { machines, loading as machinesLoading, loadMachines } from '$lib/stores/machines';
  import Spinner from '$lib/components/Spinner.svelte';

  onMount(() => {
    loadClusters();
    loadMachines();
  });
</script>

<div class="overview">
  <h1>Overview</h1>

  {#if $loading || $machinesLoading}
    <Spinner />
  {:else if $error}
    <div class="error">{$error}</div>
  {:else}
    <div class="stats-grid">
      <div class="stat-card">
        <span class="stat-value">{$clusterCount}</span>
        <span class="stat-label">Clusters</span>
      </div>
      <div class="stat-card">
        <span class="stat-value">{$machines.length}</span>
        <span class="stat-label">Machines</span>
      </div>
    </div>

    <h2>Clusters</h2>
    {#if $clusters.length === 0}
      <p class="empty">No clusters yet. <a href="/clusters/import">Import one</a></p>
    {:else}
      <div class="cluster-list">
        {#each $clusters as cluster (cluster.id)}
          <a href="/clusters/{cluster.id}" class="cluster-item">
            <span class="cluster-name">{cluster.name}</span>
            <span class="cluster-status {cluster.status}">{cluster.status}</span>
          </a>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .overview h1 { margin: 0 0 1.5rem; }
  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }
  .stat-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .stat-value {
    font-size: 2rem;
    font-weight: 700;
    color: var(--tcs-secondary);
  }
  .stat-label {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty {
    color: var(--tcs-text-muted);
  }
  .cluster-list { display: flex; flex-direction: column; gap: 0.5rem; }
  .cluster-item {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    transition: background 0.15s;
  }
  .cluster-item:hover {
    background: var(--tcs-surface-hover);
    text-decoration: none;
  }
  .cluster-name { font-weight: 500; }
  .cluster-status {
    font-size: 0.75rem;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    text-transform: capitalize;
  }
  .cluster-status.running { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .cluster-status.scalingup { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
  .cluster-status.destroying { background: rgba(239, 68, 68, 0.2); color: var(--tcs-error); }
</style>
