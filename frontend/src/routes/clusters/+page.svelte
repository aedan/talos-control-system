<script lang="ts">
  import { onMount } from 'svelte';
  import { clusters, loading, error, loadClusters, deleteCluster } from '$lib/stores/clusters';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  onMount(loadClusters);
  
  async function handleDelete(cluster: any) {
    if (!confirm(`Delete cluster "${cluster.name}"?`)) return;
    try {
      await deleteCluster(cluster.id);
      success('Cluster deleted');
    } catch {
      notifyError('Failed to delete cluster');
    }
  }
  
  function formatStatus(status: string): string {
    return status.replace(/([A-Z])/g, ' $1').replace(/^./, (s) => s.toUpperCase());
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
        <Button variant="primary">Create Cluster</Button>
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
      <a href="/clusters/create">Create your first cluster</a>
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
            <td>{cluster.controlPlaneVersion}</td>
            <td>{cluster.talosVersion}</td>
            <td>{cluster.controlPlaneNodes + cluster.workerNodes}</td>
            <td>{new Date(cluster.createdAt).toLocaleDateString()}</td>
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
  .data-table tr:hover { background: var(--tcs-surface-hover); }
  .data-table td a { color: var(--tcs-secondary); }
  .data-table td a:hover { text-decoration: underline; }
  
  .status-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    display: inline-block;
  }
  .status-badge.running { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .status-badge.scalingup { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
  .status-badge.scalingdown { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
  .status-badge.destroying { background: rgba(239, 68, 68, 0.2); color: var(--tcs-error); }
  .status-badge.unknown { background: rgba(160, 160, 160, 0.2); color: var(--tcs-text-muted); }
</style>
