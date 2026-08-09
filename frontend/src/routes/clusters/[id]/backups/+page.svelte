<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { formatBytes, isControlPlane, machineLabel, type ClusterBackup, type Machine } from '$lib/api/types';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  let backups = $state<ClusterBackup[]>([]);
  let machines = $state<Machine[]>([]);
  let loading = $state(true);
  let error = $state('');
  let creating = $state(false);
  let restoringId = $state<string | null>(null);
  let restoreMachineId = $state('');
  let scheduleHours = $state(0);
  let retention = $state(10);
  let savingSchedule = $state(false);

  async function reload() {
    const id = $page.params.id;
    const [b, m, c] = await Promise.all([
      client.get(`/clusters/${id}/backups`) as Promise<ClusterBackup[]>,
      client.get(`/clusters/${id}/machines`) as Promise<Machine[]>,
      client.get(`/clusters/${id}`) as Promise<{
        backupScheduleHours?: number | null;
        backupRetention?: number | null;
      }>,
    ]);
    backups = Array.isArray(b) ? b : [];
    machines = Array.isArray(m) ? m : [];
    scheduleHours = c.backupScheduleHours ?? 0;
    retention = c.backupRetention ?? 10;
    const cps = machines.filter(isControlPlane);
    if (cps.length && !restoreMachineId) {
      restoreMachineId = cps[0].id;
    }
  }

  onMount(async () => {
    try {
      await reload();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load backups';
    } finally {
      loading = false;
    }
  });

  async function createBackup() {
    creating = true;
    try {
      const backup = (await client.post(`/clusters/${$page.params.id}/backups`, {
        name: `etcd-${Date.now()}`,
      })) as ClusterBackup;
      backups = [backup, ...backups];
      success(backup.status === 'ready' ? 'Etcd snapshot created' : `Backup status: ${backup.status}`);
    } catch (e: unknown) {
      notifyError(
        e instanceof Error
          ? e.message
          : 'Failed to create etcd snapshot (need talosconfig + control-plane reachability)'
      );
    } finally {
      creating = false;
    }
  }

  async function downloadBackup(backup: ClusterBackup) {
    try {
      const token = localStorage.getItem('tcs_token');
      const res = await fetch(
        `/api/clusters/${$page.params.id}/backups/${backup.id}/download`,
        { headers: token ? { Authorization: `Bearer ${token}` } : {} }
      );
      if (!res.ok) throw new Error(await res.text());
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

  async function deleteBackup(backup: ClusterBackup) {
    if (!confirm(`Delete backup "${backup.name}"?`)) return;
    try {
      await client.delete(`/clusters/${$page.params.id}/backups/${backup.id}`);
      backups = backups.filter((b) => b.id !== backup.id);
      success('Backup deleted');
    } catch {
      notifyError('Failed to delete backup');
    }
  }

  async function restoreBackup(backup: ClusterBackup) {
    const ok = confirm(
      `DISASTER RECOVERY\n\nRestore etcd snapshot "${backup.name}"?\n` +
        `Target machine: ${restoreMachineId || 'auto'}\n\n` +
        `Uploads snapshot (EtcdRecover) and runs Bootstrap(recover_etcd).`
    );
    if (!ok) return;
    if (prompt('Type RESTORE to confirm:') !== 'RESTORE') {
      notifyError('Restore cancelled');
      return;
    }
    restoringId = backup.id;
    try {
      const res = (await client.post(
        `/clusters/${$page.params.id}/backups/${backup.id}/restore`,
        {
          confirm: true,
          runBootstrap: true,
          skipHashCheck: false,
          machineId: restoreMachineId || null,
        }
      )) as { ok?: boolean; message?: string; bootstrapError?: string };
      if (res.ok === false || res.bootstrapError) {
        notifyError(res.bootstrapError || res.message || 'Restore completed with errors');
      } else {
        success(res.message || 'Etcd restore requested');
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Restore failed');
    } finally {
      restoringId = null;
    }
  }

  async function saveSchedule() {
    savingSchedule = true;
    try {
      await client.put(`/clusters/${$page.params.id}/backups/schedule`, {
        scheduleHours: scheduleHours > 0 ? scheduleHours : null,
        retention: retention > 0 ? retention : 10,
      });
      success(
        scheduleHours > 0
          ? `Auto-backup every ${scheduleHours}h (keep ${retention})`
          : 'Auto-backup disabled'
      );
      await reload();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save schedule');
    } finally {
      savingSchedule = false;
    }
  }

  let controlPlanes = $derived(machines.filter(isControlPlane));
</script>

<div class="backups-page">
  <div class="page-header">
    <h1>Backups</h1>
    <Button variant="primary" onclick={createBackup} disabled={creating}>
      {creating ? 'Creating...' : 'Create Backup'}
    </Button>
  </div>

  <section class="panel">
    <h2>Schedule</h2>
    <p class="hint">Automatic etcd snapshots when the cluster has a talosconfig. Scheduler checks about every 15 minutes.</p>
    <div class="row">
      <label>
        Interval (hours, 0 = off)
        <input type="number" min="0" max="168" bind:value={scheduleHours} />
      </label>
      <label>
        Retention (ready backups to keep)
        <input type="number" min="1" max="100" bind:value={retention} />
      </label>
      <Button variant="secondary" onclick={saveSchedule} disabled={savingSchedule}>
        {savingSchedule ? 'Saving…' : 'Save schedule'}
      </Button>
    </div>
  </section>

  <section class="panel">
    <h2>Restore target</h2>
    <p class="hint">Control-plane node used for disaster recovery restore.</p>
    <select bind:value={restoreMachineId}>
      <option value="">Auto (first control-plane / talosconfig endpoint)</option>
      {#each controlPlanes as m (m.id)}
        <option value={m.id}>{machineLabel(m)} — {m.address || 'no address'}</option>
      {/each}
    </select>
  </section>

  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if backups.length === 0}
    <div class="empty-state">
      <p>No etcd snapshots yet</p>
      <p class="hint">Requires talosconfig and reachability to a control-plane :50000.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
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
            <td>{formatBytes(backup.sizeBytes)}</td>
            <td><span class="status-badge {backup.status}">{backup.status}</span></td>
            <td>{backup.createdAt ? new Date(backup.createdAt).toLocaleString() : '—'}</td>
            <td>
              <div class="actions">
                <Button variant="ghost" size="sm" onclick={() => downloadBackup(backup)} disabled={backup.status !== 'ready'}>
                  Download
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onclick={() => restoreBackup(backup)}
                  disabled={backup.status !== 'ready' || restoringId === backup.id}
                >
                  {restoringId === backup.id ? 'Restoring…' : 'Restore'}
                </Button>
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
  .panel {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin-bottom: 1rem;
  }
  .panel h2 { margin: 0 0 0.5rem; font-size: 1rem; }
  .hint { font-size: 0.85rem; color: var(--tcs-text-muted); margin: 0 0 0.75rem; }
  .row { display: flex; flex-wrap: wrap; gap: 1rem; align-items: end; }
  label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; }
  input, select {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
    min-width: 10rem;
  }
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
  .actions { display: flex; flex-wrap: wrap; gap: 0.35rem; }
  .status-badge {
    font-size: 0.75rem;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
  }
</style>
