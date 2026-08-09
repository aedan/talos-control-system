<script lang="ts">
  import { onMount } from 'svelte';
  import { clusters, loading, error, loadClusters, deleteCluster } from '$lib/stores/clusters';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { clusterNodeCount } from '$lib/api/types';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  onMount(loadClusters);

  async function handleDelete(cluster: { id: string; name: string }) {
    if (!confirm(`Delete cluster "${cluster.name}"?`)) return;
    try {
      await deleteCluster(cluster.id);
      success('Cluster deleted');
    } catch {
      notifyError('Failed to delete cluster');
    }
  }

  function formatStatus(status: string): string {
    return (status || 'unknown').replace(/_/g, ' ');
  }
</script>

<div class="clusters-page">
  <div class="page-header">
    <h1>Clusters</h1>
    <div class="header-actions">
      <a href="/clusters/import">
        <Button variant="ghost">Import Cluster</Button>
      </a>
      <a href="/clusters/create">
        <Button variant="primary">Add inventory</Button>
      </a>
    </div>
  </div>

  {#if $loading}
    <Spinner />
  {:else if $error}
    <div class="error">{$error}</div>
  {:else if $clusters.length === 0}
    <div class="empty-state">
      <p>No clusters yet</p>
      <a href="/clusters/import">Import a Talos cluster</a>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Status</th>
          <th>K8s</th>
          <th>Talos</th>
          <th>Nodes</th>
          <th>Created</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each $clusters as cluster (cluster.id)}
          <tr>
            <td><a href="/clusters/{cluster.id}">{cluster.name}</a></td>
            <td><span class="status-badge {cluster.status}">{formatStatus(cluster.status)}</span></td>
            <td>{cluster.controlPlaneVersion || '—'}</td>
            <td>{cluster.talosVersion || '—'}</td>
            <td>{clusterNodeCount(cluster)}</td>
            <td>{cluster.createdAt ? new Date(cluster.createdAt).toLocaleDateString() : '—'}</td>
            <td>
              <Button variant="danger" size="sm" onclick={() => handleDelete(cluster)}>Delete</Button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .clusters-page h1 { margin: 0; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty-state { text-align: center; padding: 3rem; color: var(--tcs-text-muted); }
  .empty-state a { color: var(--tcs-secondary); }
  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .status-badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    text-transform: capitalize;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
</style>
