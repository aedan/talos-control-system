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
    type: 'etcd-snapshot' | 'k8s-backup';
    size: number;
    status: 'ready' | 'creating' | 'failed';
    createdAt: string;
    expiresAt: string;
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
        name: `backup-${Date.now()}`,
        type: 'etcd-snapshot'
      }) as Backup;
      backups = [backup, ...backups];
      success('Backup created');
    } catch {
      notifyError('Failed to create backup');
    } finally {
      creating = false;
    }
  }
  
  async function downloadBackup(backup: Backup) {
    try {
      const res = await fetch(`/api/clusters/${$page.params.id}/backups/${backup.id}/download`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${backup.name}.tar.gz`;
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      notifyError('Failed to download backup');
    }
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
      <p>No backups yet</p>
      <p class="hint">Create a backup to safeguard your cluster's etcd data.</p>
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
          <th>Expires</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each backups as backup (backup.id)}
          <tr>
            <td>{backup.name}</td>
            <td><span class="type-badge">{backup.type}</span></td>
            <td>{formatSize(backup.size)}</td>
            <td><span class="status-badge {backup.status}">{backup.status}</span></td>
            <td>{new Date(backup.createdAt).toLocaleString()}</td>
            <td>{backup.expiresAt ? new Date(backup.expiresAt).toLocaleDateString() : 'Never'}</td>
            <td>
              <div class="actions">
                <Button variant="ghost" size="sm" onclick={() => downloadBackup(backup)}>Download</Button>
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
