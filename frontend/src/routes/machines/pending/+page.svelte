<script lang="ts">
  import { onMount } from 'svelte';
  import { machines, loading, error, loadMachines } from '$lib/stores/machines';
  import Spinner from '$lib/components/Spinner.svelte';
  
  onMount(loadMachines);
  
  let pendingList = $derived.by(() => $machines.filter(m => m.status === 'pending'));
  
  function formatMemory(bytes: number): string {
    if (bytes < 1024 * 1024) return `${bytes / 1024} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
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
      <div class="icon">
        <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="var(--tcs-success)" stroke-width="1.5">
          <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
          <polyline points="22 4 12 14.01 9 11.01"/>
        </svg>
      </div>
      <h2>All caught up</h2>
      <p>No machines are waiting to be assigned. All connected machines have been provisioned.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Machine</th>
          <th>Type</th>
          <th>Talos</th>
          <th>IP</th>
          <th>CPU</th>
          <th>Memory</th>
          <th>Arch</th>
          <th>Since</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each pendingList as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">
                {machine.hostname || machine.systemUuid.slice(0, 8)}
              </a>
            </td>
            <td><span class="type-badge">{machine.machineType}</span></td>
            <td>{machine.talosVersion}</td>
            <td>{machine.ip || '—'}</td>
            <td>{machine.cpuCores}</td>
            <td>{formatMemory(machine.memoryBytes)}</td>
            <td>{machine.arch}</td>
            <td>{new Date(machine.createdAt).toLocaleString()}</td>
            <td>
              <a href="/clusters/create?machine={machine.id}" class="assign-link">Assign</a>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .pending-page h1 { margin: 0; }
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.5rem;
  }
  .count {
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
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
    padding: 4rem 2rem;
  }
  .icon { margin-bottom: 1.5rem; }
  .empty-state h2 {
    margin: 0 0 0.5rem;
    font-size: 1.5rem;
    font-weight: 600;
  }
  .empty-state p {
    color: var(--tcs-text-muted);
    max-width: 400px;
    margin: 0 auto;
    line-height: 1.6;
  }
  
  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.875rem;
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .data-table tr:hover { background: var(--tcs-surface-hover); }
  .data-table td a { color: var(--tcs-secondary); }
  .data-table td a:hover { text-decoration: underline; }
  
  .type-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
  
  .assign-link {
    color: var(--tcs-secondary);
    font-weight: 500;
  }
</style>
