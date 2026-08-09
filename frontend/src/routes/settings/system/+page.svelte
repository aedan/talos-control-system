<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import Spinner from '$lib/components/Spinner.svelte';
  import { formatBytes } from '$lib/api/types';

  interface SystemInfo {
    version: string;
    commit: string;
    buildTime: string;
    databaseBackend: string;
    databaseSizeBytes: number | null;
    uptimeSeconds: number;
    serverBindAddr: string;
    httpPort: number;
    grpcPort: number;
    diskUsage: {
      totalBytes: number;
      freeBytes: number;
      usedBytes: number;
    };
    features?: Record<string, boolean>;
  }

  let info = $state<SystemInfo | null>(null);
  let loading = $state(true);
  let error = $state('');

  function formatUptime(seconds: number): string {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    if (days > 0) return `${days}d ${hours}h ${mins}m`;
    if (hours > 0) return `${hours}h ${mins}m`;
    return `${mins}m`;
  }

  onMount(async () => {
    try {
      info = (await client.get('/settings/system/info')) as SystemInfo;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load system info';
    } finally {
      loading = false;
    }
  });
</script>

<div class="system-page">
  <h1>System Information</h1>
  <p class="description">Runtime metrics and alpha feature flags.</p>

  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if info}
    <div class="info-grid">
      <div class="info-card">
        <h3>Application</h3>
        <div class="info-row"><span class="label">Version</span><span class="value">{info.version}</span></div>
        <div class="info-row"><span class="label">Commit</span><span class="value mono">{info.commit}</span></div>
        <div class="info-row"><span class="label">Build</span><span class="value">{info.buildTime}</span></div>
        <div class="info-row"><span class="label">Uptime</span><span class="value">{formatUptime(info.uptimeSeconds)}</span></div>
      </div>
      <div class="info-card">
        <h3>Database</h3>
        <div class="info-row"><span class="label">Backend</span><span class="value">{info.databaseBackend}</span></div>
        <div class="info-row"><span class="label">Size</span><span class="value">{formatBytes(info.databaseSizeBytes)}</span></div>
        <div class="info-row"><span class="label">Disk free</span><span class="value">{formatBytes(info.diskUsage?.freeBytes)}</span></div>
      </div>
      <div class="info-card">
        <h3>Server</h3>
        <div class="info-row"><span class="label">Bind</span><span class="value mono">{info.serverBindAddr}:{info.httpPort}</span></div>
      </div>
      {#if info.features}
        <div class="info-card wide">
          <h3>Features</h3>
          <div class="flags">
            {#each Object.entries(info.features) as [key, on]}
              <span class="flag" class:on>{key}: {on ? 'on' : 'off'}</span>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .system-page h1 { margin: 0 0 0.5rem; }
  .description { color: var(--tcs-text-muted); margin-bottom: 1.5rem; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 1rem;
  }
  .info-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
  }
  .info-card.wide { grid-column: 1 / -1; }
  .info-card h3 { margin: 0 0 0.75rem; font-size: 0.95rem; }
  .info-row {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.35rem 0;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.9rem;
  }
  .label { color: var(--tcs-text-muted); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .flags { display: flex; flex-wrap: wrap; gap: 0.5rem; }
  .flag {
    font-size: 0.75rem;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
    color: var(--tcs-text-muted);
  }
  .flag.on { color: var(--tcs-success, #22c55e); border-color: rgba(34, 197, 94, 0.4); }
</style>
