<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { machineLabel, machineHasBmc, type Machine } from '$lib/api/types';
  import Spinner from '$lib/components/Spinner.svelte';

  let machines = $state<Machine[]>([]);
  let loading = $state(true);
  let error = $state('');

  onMount(async () => {
    try {
      machines = (await client.get(`/clusters/${$page.params.id}/machines`)) as Machine[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machines';
    } finally {
      loading = false;
    }
  });
</script>

<div class="machines-page">
  <h1>Cluster machines</h1>
  <p class="hint">
    Inventory for this cluster. Click a machine to edit role, MAC, BMC, address, or install disk.
    <a href="/machines/import">Import CSV/YAML</a>
  </p>

  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if machines.length === 0}
    <div class="empty-state">
      <p>No machines assigned to this cluster</p>
      <p class="hint">Import inventory or register machines from the provision wizard.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Status</th>
          <th>Role</th>
          <th>MAC</th>
          <th>Address</th>
          <th>BMC</th>
          <th>Talos</th>
        </tr>
      </thead>
      <tbody>
        {#each machines as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">
                {machine.hostname || machineLabel(machine)}
              </a>
            </td>
            <td><span class="status-badge">{machine.status}</span></td>
            <td>{machine.machineType}</td>
            <td class="mono">{machine.macAddress || '—'}</td>
            <td class="mono">{machine.address || '—'}</td>
            <td>{machineHasBmc(machine) ? 'yes' : '—'}</td>
            <td>{machine.talosVersion || '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .hint { color: var(--tcs-text-muted); font-size: 0.9rem; }
  .error { color: var(--tcs-error); }
  .empty-state { padding: 1.5rem; color: var(--tcs-text-muted); }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.45rem 0.5rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .status-badge {
    font-size: 0.75rem;
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
  }
</style>
