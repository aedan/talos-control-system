<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import Spinner from '$lib/components/Spinner.svelte';

  interface SystemInfo {
    version: string;
    commit: string;
    build_time: string;
    database_backend: string;
    database_size_bytes: number | null;
    uptime_seconds: number;
    server_bind_addr: string;
    http_port: number;
    grpc_port: number;
    disk_usage: {
      total_bytes: number;
      free_bytes: number;
      used_bytes: number;
    };
  }

  let info = $state<SystemInfo | null>(null);
  let loading = $state(true);
  let error = $state('');

  function formatBytes(bytes: number | null): string {
    if (bytes === null || bytes === 0) return 'N/A';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let value = bytes;
    while (value >= 1024 && i < units.length - 1) {
      value /= 1024;
      i++;
    }
    return `${value.toFixed(1)} ${units[i]}`;
  }

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
      info = await client.get('/settings/system/info') as SystemInfo;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load system info';
    } finally {
      loading = false;
    }
  });
</script>

<div class="system-page">
  <h1>System Information</h1>
  <p class="description">Runtime metrics and configuration summary.</p>

  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if info}
    <div class="info-grid">
      <div class="info-card">
        <div class="card-header">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 2L2 7l10 5 10-5-10-5z"/>
            <path d="M2 17l10 5 10-5"/>
            <path d="M2 12l10 5 10-5"/>
          </svg>
          <h3>Application</h3>
        </div>
        <div class="info-rows">
          <div class="info-row">
            <span class="label">Version</span>
            <span class="value">{info.version}</span>
          </div>
          <div class="info-row">
            <span class="label">Git Commit</span>
            <span class="value">{info.commit}</span>
          </div>
          <div class="info-row">
            <span class="label">Build Time</span>
            <span class="value">{info.build_time}</span>
          </div>
          <div class="info-row">
            <span class="label">Uptime</span>
            <span class="value">{formatUptime(info.uptime_seconds)}</span>
          </div>
        </div>
      </div>

      <div class="info-card">
        <div class="card-header">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
            <ellipse cx="12" cy="5" rx="9" ry="3"/>
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
          </svg>
          <h3>Database</h3>
        </div>
        <div class="info-rows">
          <div class="info-row">
            <span class="label">Backend</span>
            <span class="value">{info.database_backend}</span>
          </div>
          <div class="info-row">
            <span class="label">Database Size</span>
            <span class="value">{formatBytes(info.database_size_bytes)}</span>
          </div>
          <div class="info-row">
            <span class="label">Disk Total</span>
            <span class="value">{formatBytes(info.disk_usage.total_bytes)}</span>
          </div>
          <div class="info-row">
            <span class="label">Disk Free</span>
            <span class="value">{formatBytes(info.disk_usage.free_bytes)}</span>
          </div>
        </div>
      </div>

      <div class="info-card">
        <div class="card-header">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 12h-4l-3 9L9 3l-3 9H2"/>
          </svg>
          <h3>Network</h3>
        </div>
        <div class="info-rows">
          <div class="info-row">
            <span class="label">Bind Address</span>
            <span class="value">{info.server_bind_addr}</span>
          </div>
          <div class="info-row">
            <span class="label">HTTP Port</span>
            <span class="value">{info.http_port}</span>
          </div>
          <div class="info-row">
            <span class="label">gRPC Port</span>
            <span class="value">{info.grpc_port}</span>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .system-page h1 { margin: 0 0 0.25rem; }
  .description { color: var(--tcs-text-muted); margin-bottom: 2rem; }

  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 1rem;
  }

  .info-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1rem;
    color: var(--tcs-secondary);
  }

  .card-header h3 {
    margin: 0;
    font-size: 0.95rem;
    color: var(--tcs-text);
  }

  .info-rows {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .info-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--tcs-border);
  }

  .info-row:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }

  .label {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }

  .value {
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--tcs-text);
  }

  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
</style>
