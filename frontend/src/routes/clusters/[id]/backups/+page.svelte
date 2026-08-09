<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface Backup {
    id: string;
    name: string;
    status: 'ready' | 'creating' | 'failed' | string;
    size_bytes?: number;
    size?: number;
    created_at?: string;
    createdAt?: string;
    file_path?: string | null;
  }
  
  let backups = $state<Backup[]>([]);
  let loading = $state(true);
  let error = $state('');
  let creating = $state(false);
  
  onMount(async () => {
    try {
      backups = await client.get(`/clusters/${$page.params.id}/backups`) as Backup[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load backups';
    } finally {
      loading = false;
    }
  });
  
  async function createBackup() {
    creating = true;
    try {
      const backup = await client.post(`/clusters/${$page.params.id}/backups`, {
        name: `etcd-${Date.now()}`,
      }) as Backup;
      backups = [backup, ...backups];
      if (backup.status === 'ready') {
        success('Etcd snapshot created');
      } else {
        success(`Backup status: ${backup.status}`);
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create etcd snapshot (need talosconfig + control-plane reachability)');
    } finally {
      creating = false;
    }
  }
  
  async function downloadBackup(backup: Backup) {
    try {
      const token = localStorage.getItem('tcs_token');
      const res = await fetch(
        `/api/clusters/${$page.params.id}/backups/${backup.id}/download`,
        {
          headers: token ? { Authorization: `Bearer ${token}` } : {},
        }
      );
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || res.statusText);
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${backup.name}.snapshot`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to download backup');
    }
  }

  function backupSize(b: Backup): number {
    return b.size_bytes ?? b.size ?? 0;
  }
  function backupCreated(b: Backup): string {
    return b.created_at ?? b.createdAt ?? '';
  }
  
  async function deleteBackup(backup: Backup) {
    if (!confirm(`Delete backup "${backup.name}"?`)) return;
    try {
      await client.delete(`/clusters/${$page.params.id}/backups/${backup.id}`);
      backups = backups.filter(b => b.id !== backup.id);
      success('Backup deleted');
    } catch {
      notifyError('Failed to delete backup');
    }
  }
  
  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
</script>

<div class="backups-page">
  <div class="page-header">
    <h1>Backups</h1>
    <Button variant="primary" onclick={createBackup} disabled={creating}>
      {creating ? 'Creating...' : 'Create Backup'}
    </Button>
  </div>
  
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if backups.length === 0}
    <div class="empty-state">
      <p>No etcd snapshots yet</p>
      <p class="hint">
        Creates a real Talos etcd snapshot via the machine API (control-plane node).
        Requires a talosconfig on the cluster.
      </p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Type</th>
          <th>Size</th>
          <th>Status</th>
          <th>Created</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each backups as backup (backup.id)}
          <tr>
            <td>{backup.name}</td>
            <td><span class="type-badge">etcd-snapshot</span></td>
            <td>{formatSize(backupSize(backup))}</td>
            <td><span class="status-badge {backup.status}">{backup.status}</span></td>
            <td>{backupCreated(backup) ? new Date(backupCreated(backup)).toLocaleString() : '—'}</td>
            <td>
              <div class="actions">
                <Button
                  variant="ghost"
                  size="sm"
                  onclick={() => downloadBackup(backup)}
                  disabled={backup.status !== 'ready'}
                >Download</Button>
                <Button variant="danger" size="sm" onclick={() => deleteBackup(backup)}>Delete</Button>
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .backups-page h1 { margin: 0; }
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.5rem;
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
    padding: 3rem;
    color: var(--tcs-text-muted);
  }
  .empty-state .hint { font-size: 0.875rem; margin-top: 0.5rem; }
  
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
  
  .type-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
  
  .status-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    display: inline-block;
  }
  .status-badge.ready { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .status-badge.creating { background: rgba(79, 139, 255, 0.2); color: var(--tcs-secondary); }
  .status-badge.failed { background: rgba(239, 68, 68, 0.2); color: var(--tcs-error); }
  
  .actions { display: flex; gap: 0.5rem; }
</style>
