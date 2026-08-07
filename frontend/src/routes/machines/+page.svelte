<script lang="ts">
  import { onMount } from 'svelte';
  import { machines, loading, error, loadMachines } from '$lib/stores/machines';
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
      <p>No machines connected yet</p>
      <p class="hint">Boot machines with TCS kernel arguments to register them.</p>
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
          <th>IP</th>
          <th>Arch</th>
        </tr>
      </thead>
      <tbody>
        {#each $machines as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">
                {machine.hostname || machine.systemUuid.slice(0, 8)}
              </a>
            </td>
            <td><span class="status-badge {machine.status}">{machine.status}</span></td>
            <td>{machine.machineType}</td>
            <td>{machine.clusterName || '—'}</td>
            <td>{machine.talosVersion}</td>
            <td>{machine.ip || '—'}</td>
            <td>{machine.arch}</td>
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
  .status-badge.pending { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
  .status-badge.booting { background: rgba(79, 139, 255, 0.2); color: var(--tcs-secondary); }
  .status-badge.installing { background: rgba(79, 139, 255, 0.2); color: var(--tcs-secondary); }
  .status-badge.configuring { background: rgba(79, 139, 255, 0.2); color: var(--tcs-secondary); }
  .status-badge.destroying { background: rgba(239, 68, 68, 0.2); color: var(--tcs-error); }
</style>
