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
  import { machineLabel, machineHasBmc } from '$lib/api/types';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  onMount(() => {
    void loadMachines();
    startMachinesPolling();
  });
  onDestroy(stopMachinesPolling);

  let filter = $state('');

  let filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return $machines;
    return $machines.filter((m) => {
      const hay = [
        m.hostname,
        m.systemUuid,
        m.address,
        m.macAddress,
        m.bmcAddress,
        m.machineType,
        m.status,
        m.clusterId,
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return hay.includes(q);
    });
  });
</script>

<div class="machines-page">
  <div class="page-header">
    <h1>Machines</h1>
    <div class="actions">
      <a href="/machines/import"><Button variant="secondary" size="sm">Import CSV/YAML</Button></a>
      <a href="/clusters/create"><Button variant="primary" size="sm">Provision bare metal</Button></a>
    </div>
  </div>

  <div class="toolbar">
    <input type="search" placeholder="Filter hostname, MAC, address, BMC…" bind:value={filter} />
  </div>

  {#if $loading}
    <Spinner />
  {:else if $error}
    <div class="error">{$error}</div>
  {:else if $machines.length === 0}
    <div class="empty-state">
      <p>No machines yet</p>
      <p class="hint">
        Import a CSV/YAML inventory, import a running cluster via kubeconfig, or register machines
        under Clusters → Provision bare metal.
      </p>
      <a href="/machines/import"><Button variant="primary">Import inventory</Button></a>
    </div>
  {:else if filtered.length === 0}
    <div class="empty-state"><p>No machines match “{filter}”</p></div>
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
        {#each filtered as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">
                {machine.hostname || machineLabel(machine)}
              </a>
              {#if machine.hostname}
                <div class="sub mono">{machineLabel(machine)}</div>
              {/if}
            </td>
            <td><span class="status-badge {machine.status}">{machine.status}</span></td>
            <td>{machine.machineType || '—'}</td>
            <td class="mono">{machine.macAddress || '—'}</td>
            <td class="mono">{machine.address || '—'}</td>
            <td>
              {#if machineHasBmc(machine)}
                <span class="bmc-ok" title={machine.bmcAddress}>BMC</span>
              {:else}
                <span class="muted">—</span>
              {/if}
            </td>
            <td>{machine.talosVersion || '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .machines-page h1 { margin: 0; }
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
  }
  .actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .toolbar { margin-bottom: 1rem; }
  .toolbar input {
    width: min(28rem, 100%);
    padding: 0.45rem 0.6rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
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
    padding: 2rem;
    color: var(--tcs-text-muted);
  }
  .hint { font-size: 0.9rem; max-width: 36rem; margin: 0.5rem auto 1rem; }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .sub { color: var(--tcs-text-muted); font-size: 0.75rem; }
  .bmc-ok {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
    background: rgba(79, 139, 255, 0.2);
    color: var(--tcs-secondary);
    font-size: 0.75rem;
  }
  .muted { color: var(--tcs-text-muted); }
  .status-badge {
    font-size: 0.75rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
</style>
