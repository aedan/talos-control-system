<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { machineClasses, loading, error as storeError, loadMachineClasses, createMachineClass, deleteMachineClass } from '$lib/stores/machine-classes';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  onMount(loadMachineClasses);

  let showForm = $state(false);
  let formName = $state('');
  let formDescription = $state('');
  let formMinCpu = $state(2);
  let formMinMemory = $state(8589934592);
  let formMinDisk = $state(42949672960);
  let formArch = $state('x86_64');
  let formSecureBoot = $state(true);
  let formControlPlane = $state(true);
  let formWorker = $state(true);
  let formError = $state('');

  async function handleCreate() {
    if (!formName.trim()) {
      formError = 'Name is required';
      return;
    }

    const allowedRoles: string[] = [];
    if (formControlPlane) allowedRoles.push('control-plane');
    if (formWorker) allowedRoles.push('worker');

    try {
      await createMachineClass({
        name: formName.trim(),
        description: formDescription.trim(),
        minCpu: formMinCpu,
        minMemory: formMinMemory,
        minDisk: formMinDisk,
        arch: formArch,
        secureBoot: formSecureBoot,
        allowedRoles,
      });
      success('Machine class created');
      resetForm();
    } catch {
      notifyError('Failed to create machine class');
    }
  }

  function resetForm() {
    showForm = false;
    formName = '';
    formDescription = '';
    formMinCpu = 2;
    formMinMemory = 8589934592;
    formMinDisk = 42949672960;
    formArch = 'x86_64';
    formSecureBoot = true;
    formControlPlane = true;
    formWorker = true;
    formError = '';
  }

  async function handleDelete(mc: any) {
    if (!confirm(`Delete machine class "${mc.name}"?`)) return;
    try {
      await deleteMachineClass(mc.id);
      success('Machine class deleted');
    } catch {
      notifyError('Failed to delete machine class');
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
</script>

<div class="machine-classes-page">
  <div class="page-header">
    <h1>Machine Classes</h1>
    <div class="header-actions">
      {#if showForm}
        <Button variant="ghost" onclick={resetForm}>Cancel</Button>
      {:else}
        <Button variant="primary" onclick={() => { showForm = true; }}>Create Machine Class</Button>
      {/if}
    </div>
  </div>

  {#if showForm}
    <div class="create-form">
      <div class="form-group">
        <label for="mc-name">Name</label>
        <input id="mc-name" type="text" bind:value={formName} placeholder="e.g. Standard Worker" />
      </div>
      <div class="form-group">
        <label for="mc-desc">Description</label>
        <input id="mc-desc" type="text" bind:value={formDescription} placeholder="Optional description" />
      </div>
      <div class="form-row">
        <div class="form-group">
          <label for="mc-cpu">Min CPU Cores</label>
          <input id="mc-cpu" type="number" min="1" bind:value={formMinCpu} />
        </div>
        <div class="form-group">
          <label for="mc-mem">Min Memory</label>
          <select bind:value={formMinMemory}>
            <option value={4 * 1024 * 1024 * 1024}>4 GB</option>
            <option value={8 * 1024 * 1024 * 1024}>8 GB</option>
            <option value={16 * 1024 * 1024 * 1024}>16 GB</option>
            <option value={32 * 1024 * 1024 * 1024}>32 GB</option>
            <option value={64 * 1024 * 1024 * 1024}>64 GB</option>
            <option value={128 * 1024 * 1024 * 1024}>128 GB</option>
          </select>
        </div>
        <div class="form-group">
          <label for="mc-disk">Min Disk</label>
          <select bind:value={formMinDisk}>
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
          <label for="mc-arch">Architecture</label>
          <select id="mc-arch" bind:value={formArch}>
            <option value="x86_64">x86_64</option>
            <option value="aarch64">aarch64</option>
          </select>
        </div>
        <div class="form-group">
          <label>
            <input type="checkbox" bind:checked={formSecureBoot} />
            Secure Boot
          </label>
        </div>
        <div class="form-group">
          <label>Allowed Roles</label>
          <div class="checkbox-group">
            <label>
              <input type="checkbox" bind:checked={formControlPlane} />
              Control Plane
            </label>
            <label>
              <input type="checkbox" bind:checked={formWorker} />
              Worker
            </label>
          </div>
        </div>
      </div>
      {#if formError}
        <div class="form-error">{formError}</div>
      {/if}
      <div class="form-actions">
        <Button variant="primary" onclick={handleCreate}>Create</Button>
      </div>
    </div>
  {/if}

  {#if $loading}
    <Spinner />
  {:else if $storeError}
    <div class="error">{$storeError}</div>
  {:else if $machineClasses.length === 0}
    <div class="empty-state">
      <p>No machine classes yet</p>
      <Button variant="ghost" onclick={() => { showForm = true; }}>Create your first machine class</Button>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Description</th>
          <th>Arch</th>
          <th>Min CPU</th>
          <th>Min Memory</th>
          <th>Min Disk</th>
          <th>Roles</th>
          <th>Secure Boot</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each $machineClasses as mc (mc.id)}
          <tr>
            <td><a href="/machine-classes/{mc.id}">{mc.name}</a></td>
            <td class="description-cell">{mc.description || '—'}</td>
            <td><span class="arch-badge">{mc.arch}</span></td>
            <td>{mc.minCpu} cores</td>
            <td>{formatBytes(mc.minMemory)}</td>
            <td>{formatBytes(mc.minDisk)}</td>
            <td>
              {#each mc.allowedRoles as role}
                <span class="role-tag">{role}</span>
              {/each}
            </td>
            <td>{mc.secureBoot ? 'Yes' : 'No'}</td>
            <td>
              <Button variant="danger" size="sm" onclick={() => handleDelete(mc)}>Delete</Button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .machine-classes-page h1 { margin: 0; }
  .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; }
  .header-actions { display: flex; gap: 0.5rem; }

  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }

  .empty-state { text-align: center; padding: 3rem; color: var(--tcs-text-muted); }
  .empty-state p { margin-bottom: 1rem; }

  .create-form {
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

  .form-actions { display: flex; justify-content: flex-end; }
  .form-error { color: var(--tcs-error); font-size: 0.85rem; margin-bottom: 0.75rem; }

  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .data-table tr:hover { background: var(--tcs-surface-hover); }
  .data-table td a { color: var(--tcs-secondary); }
  .data-table td a:hover { text-decoration: underline; }

  .description-cell { max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--tcs-text-muted); font-size: 0.875rem; }

  .arch-badge {
    font-size: 0.7rem;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    font-family: monospace;
  }

  .role-tag {
    font-size: 0.7rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: var(--tcs-primary);
    color: white;
    margin-right: 0.25rem;
  }
</style>
