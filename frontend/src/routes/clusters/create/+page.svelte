<script lang="ts">
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { goto } from '$app/navigation';
  import Button from '$lib/components/Button.svelte';
  
  let name = '';
  let controlPlaneVersion = 'v1.31.0';
  let talosVersion = 'v1.8.0';
  let controlPlaneSize = 3;
  let workerSize = 3;
  let creating = false;
  
  async function handleCreate() {
    if (!name.trim()) return;
    creating = true;
    try {
      // Inventory only — alpha does not provision Talos/Kubernetes clusters.
      await client.post('/clusters', {
        name: name.trim(),
        control_plane_version: controlPlaneVersion,
        talos_version: talosVersion,
      });
      success('Inventory record created. Use Import for real clusters.');
      goto('/clusters');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create cluster record');
    } finally {
      creating = false;
    }
  }
</script>

<div class="create-page">
  <h1>Create inventory record</h1>
  <p class="hint" style="opacity:0.85;margin-bottom:1rem;">
    Alpha does <strong>not</strong> provision Talos clusters. Prefer
    <a href="/clusters/import">Import</a> with kubeconfig + talosconfig for real environments.
    This form only inserts a placeholder row in the TCS database.
  </p>
  
  <form class="create-form" onsubmit={(e) => { e.preventDefault(); handleCreate(); }}>
    <div class="form-group">
      <label for="name">Cluster Name</label>
      <input id="name" type="text" bind:value={name} placeholder="my-cluster" required />
    </div>
    
    <div class="form-row">
      <div class="form-group">
        <label for="k8sVersion">Kubernetes Version</label>
        <select id="k8sVersion" bind:value={controlPlaneVersion}>
          <option value="v1.31.0">v1.31.0</option>
          <option value="v1.30.4">v1.30.4</option>
          <option value="v1.29.8">v1.29.8</option>
        </select>
      </div>
      
      <div class="form-group">
        <label for="talosVersion">Talos Version</label>
        <select id="talosVersion" bind:value={talosVersion}>
          <option value="v1.8.0">v1.8.0</option>
          <option value="v1.7.5">v1.7.5</option>
          <option value="v1.6.8">v1.6.8</option>
        </select>
      </div>
    </div>
    
    <div class="form-row">
      <div class="form-group">
        <label for="cpSize">Control Plane Nodes</label>
        <input id="cpSize" type="number" bind:value={controlPlaneSize} min="1" max="10" step="2" />
      </div>
      
      <div class="form-group">
        <label for="workerSize">Worker Nodes</label>
        <input id="workerSize" type="number" bind:value={workerSize} min="0" max="50" />
      </div>
    </div>
    
    <div class="form-actions">
      <Button variant="ghost" type="button" onclick={() => window.history.back()}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={creating}>
        {creating ? 'Creating...' : 'Create Cluster'}
      </Button>
    </div>
  </form>
</div>

<style>
  .create-page h1 { margin: 0 0 1.5rem; }
  .create-form { max-width: 600px; display: flex; flex-direction: column; gap: 1.5rem; }
  .form-group { display: flex; flex-direction: column; gap: 0.4rem; flex: 1; }
  .form-row { display: flex; gap: 1rem; }
  .form-group label { color: var(--tcs-text-muted); font-size: 0.875rem; }
  .form-group input, .form-group select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    outline: none;
    transition: border-color 0.15s;
  }
  .form-group input:focus, .form-group select:focus {
    border-color: var(--tcs-primary);
  }
  .form-actions { display: flex; gap: 1rem; justify-content: flex-end; margin-top: 1rem; }
</style>
