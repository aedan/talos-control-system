<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface Machine {
    id: string;
    systemUuid: string;
    machineType: 'controlplane' | 'worker';
    clusterId: string | null;
    clusterName: string | null;
    status: string;
    talosVersion: string;
    kubernetesVersion: string | null;
    hostname: string | null;
    arch: string;
    memoryBytes: number;
    cpuCores: number;
    diskBytes: number;
    ip: string | null;
    secureBoot: boolean;
    siderolinkConnected: boolean;
    createdAt: string;
    updatedAt: string;
  }
  
  let machine = $state<Machine | null>(null);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      machine = await client.get(`/machines/${$page.params.id}`) as Machine;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine';
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

<div class="machine-detail">
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if machine}
    <div class="detail-header">
      <h1>{machine.hostname || machine.systemUuid.slice(0, 8)}</h1>
      <div class="header-actions">
        <span class="status-badge {machine.status}">{machine.status}</span>
        <span class="type-badge">{machine.machineType}</span>
      </div>
    </div>
    
    <div class="info-grid">
      <div class="info-section">
        <h2>System</h2>
        <div class="info-row">
          <span class="label">Hostname</span>
          <span class="value">{machine.hostname || '—'}</span>
        </div>
        <div class="info-row">
          <span class="label">System UUID</span>
          <span class="value mono">{machine.systemUuid}</span>
        </div>
        <div class="info-row">
          <span class="label">Architecture</span>
          <span class="value">{machine.arch}</span>
        </div>
        <div class="info-row">
          <span class="label">IP Address</span>
          <span class="value mono">{machine.ip || '—'}</span>
        </div>
        <div class="info-row">
          <span class="label">Secure Boot</span>
          <span class="value">{machine.secureBoot ? 'Yes' : 'No'}</span>
        </div>
      </div>
      
      <div class="info-section">
        <h2>Resources</h2>
        <div class="info-row">
          <span class="label">CPU Cores</span>
          <span class="value">{machine.cpuCores}</span>
        </div>
        <div class="info-row">
          <span class="label">Memory</span>
          <span class="value">{formatBytes(machine.memoryBytes)}</span>
        </div>
        <div class="info-row">
          <span class="label">Disk</span>
          <span class="value">{formatBytes(machine.diskBytes)}</span>
        </div>
      </div>
      
      <div class="info-section">
        <h2>Software</h2>
        <div class="info-row">
          <span class="label">Talos Version</span>
          <span class="value mono">{machine.talosVersion}</span>
        </div>
        <div class="info-row">
          <span class="label">Kubernetes Version</span>
          <span class="value mono">{machine.kubernetesVersion || '—'}</span>
        </div>
        <div class="info-row">
          <span class="label">Cluster</span>
          <span class="value">
            {#if machine.clusterId}
              <a href="/clusters/{machine.clusterId}">{machine.clusterName}</a>
            {:else}
              Unassigned
            {/if}
          </span>
        </div>
      </div>
      
      <div class="info-section">
        <h2>Connection</h2>
        <div class="info-row">
          <span class="label">Siderolink</span>
          <span class="value">
            <span class="connection-dot {machine.siderolinkConnected ? 'connected' : 'disconnected'}"></span>
            {machine.siderolinkConnected ? 'Connected' : 'Disconnected'}
          </span>
        </div>
        <div class="info-row">
          <span class="label">Registered</span>
          <span class="value">{new Date(machine.createdAt).toLocaleString()}</span>
        </div>
        <div class="info-row">
          <span class="label">Last Updated</span>
          <span class="value">{new Date(machine.updatedAt).toLocaleString()}</span>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .machine-detail h1 { margin: 0; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  
  .detail-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 2rem;
  }
  
  .header-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  
  .status-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
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
  
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1.5rem;
  }
  
  .info-section {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
  }
  
  .info-section h2 {
    margin: 0 0 1rem;
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  
  .info-row {
    display: flex;
    justify-content: space-between;
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.875rem;
  }
  
  .info-row:last-child {
    border-bottom: none;
  }
  
  .label {
    color: var(--tcs-text-muted);
  }
  
  .value {
    font-weight: 500;
  }
  
  .value.mono {
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 0.8rem;
  }
  
  .value a {
    color: var(--tcs-secondary);
  }
  
  .connection-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 0.4rem;
    vertical-align: middle;
  }
  
  .connection-dot.connected {
    background: var(--tcs-success);
    box-shadow: 0 0 6px var(--tcs-success);
  }
  
  .connection-dot.disconnected {
    background: var(--tcs-error);
    box-shadow: 0 0 6px var(--tcs-error);
  }
</style>
