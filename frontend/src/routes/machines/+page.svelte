<script lang="ts">
  import { onMount } from 'svelte';
  import { machines, loading, error, loadMachines } from '$lib/stores/machines';
  import { machineLabel } from '$lib/api/types';
  import Spinner from '$lib/components/Spinner.svelte';

  onMount(loadMachines);
</script>

<div class="machines-page">
  <div class="page-header">
    <h1>Machines</h1>
  </div>

  {#if $loading}
    <Spinner />
  {:else if $error}
    <div class="error">{$error}</div>
  {:else if $machines.length === 0}
    <div class="empty-state">
      <p>No machines yet</p>
      <p class="hint">Import a cluster with kubeconfig to inventory nodes, or register machines later via discovery.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Status</th>
          <th>Type</th>
          <th>Cluster</th>
          <th>Talos</th>
          <th>Address</th>
        </tr>
      </thead>
      <tbody>
        {#each $machines as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">{machineLabel(machine)}</a>
            </td>
            <td><span class="status-badge {machine.status}">{machine.status}</span></td>
            <td>{machine.machineType || '—'}</td>
            <td>{machine.clusterId ? machine.clusterId.slice(0, 8) : '—'}</td>
            <td>{machine.talosVersion || '—'}</td>
            <td class="mono">{machine.address || '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .machines-page h1 { margin: 0; }
  .page-header { margin-bottom: 1.5rem; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty-state { text-align: center; padding: 3rem; color: var(--tcs-text-muted); }
  .empty-state .hint { font-size: 0.875rem; margin-top: 0.5rem; }
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
  .mono { font-family: ui-monospace, monospace; font-size: 0.85rem; }
  .status-badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
</style>
