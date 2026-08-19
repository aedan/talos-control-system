<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    machines,
    loading,
    error,
    loadMachines,
    startMachinesPolling,
    stopMachinesPolling,
  } from '$lib/stores/machines';
  import { machineLabel } from '$lib/api/types';
  import Spinner from '$lib/components/Spinner.svelte';

  onMount(() => {
    void loadMachines();
    startMachinesPolling();
  });
  onDestroy(stopMachinesPolling);

  let pendingList = $derived.by(() => $machines.filter((m) => m.status === 'pending'));
</script>

<div class="pending-page">
  <div class="page-header">
    <h1>Pending Machines</h1>
    <span class="count">{pendingList.length} waiting</span>
  </div>

  {#if $loading}
    <Spinner />
  {:else if $error}
    <div class="error">{$error}</div>
  {:else if pendingList.length === 0}
    <div class="empty-state">
      <h2>All caught up</h2>
      <p>No machines are waiting. Imported nodes appear under Machines when inventory is populated.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Machine</th>
          <th>Type</th>
          <th>Talos</th>
          <th>Address</th>
          <th>Since</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each pendingList as machine (machine.id)}
          <tr>
            <td><a href="/machines/{machine.id}">{machineLabel(machine)}</a></td>
            <td><span class="type-badge">{machine.machineType || '—'}</span></td>
            <td>{machine.talosVersion || '—'}</td>
            <td class="mono">{machine.address || '—'}</td>
            <td>{machine.createdAt ? new Date(machine.createdAt).toLocaleString() : '—'}</td>
            <td>
              <a href="/machines/{machine.id}" class="assign-link">Open</a>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .pending-page h1 { margin: 0; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; }
  .count { color: var(--tcs-text-muted); font-size: 0.9rem; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty-state { text-align: center; padding: 3rem; color: var(--tcs-text-muted); }
  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.85rem; }
  .assign-link { color: var(--tcs-secondary); }
  .type-badge {
    font-size: 0.75rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
  }
</style>
