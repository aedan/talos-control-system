<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { client } from '$lib/api/client';
  import { onMount } from 'svelte';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  interface MachineClass {
    id: string;
    name: string;
    description: string;
    minCpu: number;
    minMemory: number;
    minDisk: number;
    arch: string;
    secureBoot: boolean;
    allowedRoles: string[];
    createdAt: string;
    updatedAt: string;
  }

  let machineClass = $state<MachineClass | null>(null);
  let loading = $state(true);
  let error = $state('');
  let editing = $state(false);
  let editName = $state('');
  let editDescription = $state('');
  let editMinCpu = $state(2);
  let editMinMemory = $state(8589934592);
  let editMinDisk = $state(42949672960);
  let editArch = $state('x86_64');
  let editSecureBoot = $state(true);
  let editControlPlane = $state(true);
  let editWorker = $state(false);
  let saving = $state(false);

  onMount(async () => {
    try {
      const data = await client.get(`/machine-classes/${$page.params.id}`) as MachineClass;
      machineClass = data;
      loadEditForm(data);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine class';
    } finally {
      loading = false;
    }
  });

  function loadEditForm(mc: MachineClass) {
    editName = mc.name;
    editDescription = mc.description;
    editMinCpu = mc.minCpu;
    editMinMemory = mc.minMemory;
    editMinDisk = mc.minDisk;
    editArch = mc.arch;
    editSecureBoot = mc.secureBoot;
    editControlPlane = mc.allowedRoles.includes('control-plane');
    editWorker = mc.allowedRoles.includes('worker');
  }

  async function handleSave() {
    if (!machineClass || !editName.trim()) return;
    saving = true;

    const allowedRoles: string[] = [];
    if (editControlPlane) allowedRoles.push('control-plane');
    if (editWorker) allowedRoles.push('worker');

    try {
      const data = await client.put(`/machine-classes/${machineClass.id}`, {
        name: editName.trim(),
        description: editDescription.trim(),
        minCpu: editMinCpu,
        minMemory: editMinMemory,
        minDisk: editMinDisk,
        arch: editArch,
        secureBoot: editSecureBoot,
        allowedRoles,
      });
      machineClass = data as MachineClass;
      editing = false;
      success('Machine class updated');
    } catch {
      notifyError('Failed to update machine class');
    } finally {
      saving = false;
    }
  }

  function cancelEdit() {
    if (machineClass) loadEditForm(machineClass);
    editing = false;
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
</script>

<div class="class-detail">
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error-banner">{error}</div>
    <Button variant="ghost" onclick={() => goto('/machine-classes')}>Back to Machine Classes</Button>
  {:else if machineClass}
    <div class="detail-header">
      <div class="header-left">
        <Button variant="ghost" onclick={() => goto('/machine-classes')}>Back to Machine Classes</Button>
        <h1>{editing ? '' : machineClass.name}</h1>
      </div>
      <Button variant="ghost" onclick={() => { editing = true; loadEditForm(machineClass!); }}>Edit</Button>
    </div>

    {#if editing}
      <div class="edit-form">
        <div class="form-row">
          <div class="form-group">
            <label for="edit-name">Name</label>
            <input id="edit-name" type="text" bind:value={editName} />
          </div>
          <div class="form-group">
            <label for="edit-desc">Description</label>
            <input id="edit-desc" type="text" bind:value={editDescription} />
          </div>
        </div>
        <div class="form-row">
          <div class="form-group">
            <label for="edit-cpu">Min CPU Cores</label>
            <input id="edit-cpu" type="number" min="1" bind:value={editMinCpu} />
          </div>
          <div class="form-group">
            <label for="edit-mem">Min Memory</label>
            <select bind:value={editMinMemory}>
              <option value={4 * 1024 * 1024 * 1024}>4 GB</option>
              <option value={8 * 1024 * 1024 * 1024}>8 GB</option>
              <option value={16 * 1024 * 1024 * 1024}>16 GB</option>
              <option value={32 * 1024 * 1024 * 1024}>32 GB</option>
              <option value={64 * 1024 * 1024 * 1024}>64 GB</option>
              <option value={128 * 1024 * 1024 * 1024}>128 GB</option>
            </select>
          </div>
          <div class="form-group">
            <label for="edit-disk">Min Disk</label>
            <select bind:value={editMinDisk}>
              <option value={20 * 1024 * 1024 * 1024}>20 GB</option>
              <option value={40 * 1024 * 1024 * 1024}>40 GB</option>
              <option value={80 * 1024 * 1024 * 1024}>80 GB</option>
              <option value={160 * 1024 * 1024 * 1024}>160 GB</option>
              <option value={320 * 1024 * 1024 * 1024}>320 GB</option>
            </select>
          </div>
        </div>
        <div class="form-row">
          <div class="form-group">
            <label for="edit-arch">Architecture</label>
            <select id="edit-arch" bind:value={editArch}>
              <option value="x86_64">x86_64</option>
              <option value="aarch64">aarch64</option>
            </select>
          </div>
          <div class="form-group">
            <label>
              <input type="checkbox" bind:checked={editSecureBoot} />
              Secure Boot
            </label>
          </div>
          <div class="form-group">
            <label>Allowed Roles</label>
            <div class="checkbox-group">
              <label>
                <input type="checkbox" bind:checked={editControlPlane} />
                Control Plane
              </label>
              <label>
                <input type="checkbox" bind:checked={editWorker} />
                Worker
              </label>
            </div>
          </div>
        </div>
        <div class="form-actions">
          <Button variant="ghost" onclick={cancelEdit} disabled={saving}>Cancel</Button>
          <Button variant="primary" onclick={handleSave} disabled={saving}>
            {saving ? 'Saving...' : 'Save Changes'}
          </Button>
        </div>
      </div>
    {:else}
      <p class="description">{machineClass.description || 'No description provided.'}</p>

      <div class="info-grid">
        <div class="info-section">
          <h2>Hardware Requirements</h2>
          <div class="info-row">
            <span class="label">Min CPU</span>
            <span class="value">{machineClass.minCpu} cores</span>
          </div>
          <div class="info-row">
            <span class="label">Min Memory</span>
            <span class="value">{formatBytes(machineClass.minMemory)}</span>
          </div>
          <div class="info-row">
            <span class="label">Min Disk</span>
            <span class="value">{formatBytes(machineClass.minDisk)}</span>
          </div>
          <div class="info-row">
            <span class="label">Architecture</span>
            <span class="value">{machineClass.arch}</span>
          </div>
        </div>

        <div class="info-section">
          <h2>Policies</h2>
          <div class="info-row">
            <span class="label">Secure Boot</span>
            <span class="value">{machineClass.secureBoot ? 'Required' : 'Optional'}</span>
          </div>
          <div class="info-row">
            <span class="label">Allowed Roles</span>
            <span class="value">
              {#each machineClass.allowedRoles as role}
                <span class="role-tag">{role}</span>
              {/each}
            </span>
          </div>
          <div class="info-row">
            <span class="label">Created</span>
            <span class="value">{new Date(machineClass.createdAt).toLocaleString()}</span>
          </div>
          <div class="info-row">
            <span class="label">Updated</span>
            <span class="value">{new Date(machineClass.updatedAt).toLocaleString()}</span>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .class-detail h1 { margin: 0; }
  .description {
    color: var(--tcs-text-muted);
    margin: 0.5rem 0 2rem;
  }

  .detail-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1.5rem;
  }
  .header-left { display: flex; flex-direction: column; gap: 0.25rem; }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
    margin-bottom: 1rem;
  }

  .edit-form {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }

  .form-group { margin-bottom: 1rem; }
  .form-group label { display: block; font-size: 0.8rem; color: var(--tcs-text-muted); margin-bottom: 0.35rem; text-transform: uppercase; letter-spacing: 0.04em; }

  .form-group input[type="text"],
  .form-group input[type="number"],
  .form-group select {
    width: 100%;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    background: var(--tcs-background);
    color: var(--tcs-text);
    font-size: 0.875rem;
  }

  .form-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 1rem;
  }

  .checkbox-group { display: flex; gap: 1rem; }
  .checkbox-group label { font-size: 0.875rem; color: var(--tcs-text); text-transform: none; letter-spacing: 0; display: flex; align-items: center; gap: 0.35rem; }
  .form-group label:has(input[type="checkbox"]) { text-transform: none; letter-spacing: 0; color: var(--tcs-text); display: flex; align-items: center; gap: 0.35rem; }

  .form-actions { display: flex; justify-content: flex-end; gap: 0.5rem; }

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

  .info-row:last-child { border-bottom: none; }
  .label { color: var(--tcs-text-muted); }
  .value { font-weight: 500; }

  .role-tag {
    font-size: 0.7rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: var(--tcs-primary);
    color: white;
    margin-right: 0.25rem;
  }
</style>
