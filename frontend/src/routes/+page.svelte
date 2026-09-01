<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    clusters,
    loading,
    error,
    clusterCount,
    loadClusters,
    deleteCluster,
    startClustersPolling,
    stopClustersPolling,
  } from '$lib/stores/clusters';
  import { machines, loading as machinesLoading, loadMachines } from '$lib/stores/machines';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { clusterNodeCount } from '$lib/api/types';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  onMount(() => {
    loadClusters();
    loadMachines();
    startClustersPolling();
  });
  onDestroy(stopClustersPolling);

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

<div class="overview">
  <div class="page-header">
    <h1>Dashboard</h1>
    <div class="header-actions">
      <a href="/clusters/import"><Button variant="primary" title="Adopt an existing Talos cluster by pasting its kubeconfig / talosconfig">Import cluster</Button></a>
      <a href="/machines/import"><Button variant="primary" title="Add a list of machines to TCS inventory (YAML or CSV) for an existing or new cluster">Add machines</Button></a>
      <a href="/clusters/create"><Button variant="primary" title="Build a new cluster from bare metal via PXE + BMC">Provision cluster</Button></a>
    </div>
  </div>

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

    {#if $clusters.length === 0}
      <div class="empty-state">
        <p>No clusters yet</p>
        <div class="empty-actions">
          <a href="/clusters/import"><Button variant="primary" title="Adopt an existing Talos cluster by pasting its kubeconfig / talosconfig">Import cluster</Button></a>
          <a href="/clusters/create"><Button variant="primary" title="Build a new cluster from bare metal via PXE + BMC">Provision cluster</Button></a>
        </div>
        <p class="empty-hint">You can also <a href="/machines/import">add machines to inventory</a> first, then import or provision.</p>
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
                <Button variant="danger" size="sm" title="Remove this cluster from TCS (does not delete the running cluster)" onclick={() => handleDelete(cluster)}>Delete</Button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</div>

<style>
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.5rem;
  }
  .overview h1 {
    margin: 0;
  }
  .header-actions {
    display: flex;
    gap: 0.5rem;
  }
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
  .empty-state {
    text-align: center;
    padding: 3rem;
    color: var(--tcs-text-muted);
  }
  .empty-state a {
    color: var(--tcs-secondary);
  }
  .empty-actions {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
    margin: 1rem 0;
  }
  .empty-hint {
    font-size: 0.875rem;
  }
  .data-table {
    width: 100%;
    border-collapse: collapse;
  }
  .data-table th,
  .data-table td {
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
