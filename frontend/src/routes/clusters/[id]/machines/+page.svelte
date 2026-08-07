<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface Machine {
    id: string;
    systemUuid: string;
    machineType: 'controlplane' | 'worker';
    status: string;
    talosVersion: string;
    hostname: string | null;
    ip: string | null;
    arch: string;
    memoryBytes: number;
    cpuCores: number;
    createdAt: string;
  }
  
  let machines = $state<Machine[]>([]);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      machines = await client.get(`/clusters/${$page.params.id}/machines`) as Machine[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machines';
    } finally {
      loading = false;
    }
  });
  
  function formatBytes(bytes: number): string {
    if (bytes < 1024 * 1024) return `${bytes / 1024} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
</script>

<div class="machines-page">
  <h1>Machines</h1>
  
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if machines.length === 0}
    <div class="empty-state">
      <p>No machines assigned to this cluster</p>
      <p class="hint">Machines will appear here once they connect via siderolink.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Machine</th>
          <th>Status</th>
          <th>Type</th>
          <th>Talos</th>
          <th>IP</th>
          <th>CPU</th>
          <th>Memory</th>
          <th>Arch</th>
          <th>Connected</th>
        </tr>
      </thead>
      <tbody>
        {#each machines as machine (machine.id)}
          <tr>
            <td>
              <a href="/machines/{machine.id}">
                {machine.hostname || machine.systemUuid.slice(0, 8)}
              </a>
            </td>
            <td><span class="status-badge {machine.status}">{machine.status}</span></td>
            <td><span class="type-badge">{machine.machineType}</span></td>
            <td>{machine.talosVersion}</td>
            <td>{machine.ip || '—'}</td>
            <td>{machine.cpuCores}</td>
            <td>{formatBytes(machine.memoryBytes)}</td>
            <td>{machine.arch}</td>
            <td>{new Date(machine.createdAt).toLocaleDateString()}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .machines-page h1 { margin: 0 0 1.5rem; }
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
  .empty-state .hint {
    font-size: 0.875rem;
    margin-top: 0.5rem;
  }
  
  .data-table {
    width: 100%;
    border-collapse: collapse;
  }
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
  
  .status-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    display: inline-block;
  }
  .status-badge.running { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .status-badge.pending { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
  .status-badge.booting,
  .status-badge.installing,
  .status-badge.configuring { background: rgba(79, 139, 255, 0.2); color: var(--tcs-secondary); }
  .status-badge.destroying { background: rgba(239, 68, 68, 0.2); color: var(--tcs-error); }
  
  .type-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
</style>
