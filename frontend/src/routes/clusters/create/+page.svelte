<script lang="ts">
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { goto } from '$app/navigation';
  import Button from '$lib/components/Button.svelte';

  let name = '';
  let endpoint = 'https://192.168.0.10:6443';
  let controlPlaneVersion = 'v1.31.0';
  let talosVersion = 'v1.9.0';
  let creating = false;
  let generating = false;
  let generated = $state<null | {
    id: string;
    controlplaneConfig?: string;
    workerConfig?: string;
    hasSecrets?: boolean;
  }>(null);
  let alsoInventory = true;

  async function handleCreate() {
    if (!name.trim()) return;
    creating = true;
    try {
      await client.post('/clusters', {
        name: name.trim(),
        controlPlaneVersion,
        talosVersion,
      });
      success('Inventory record created. Use Import for real clusters.');
      goto('/clusters');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create cluster record');
    } finally {
      creating = false;
    }
  }

  async function generateConfigs() {
    if (!name.trim() || !endpoint.trim()) {
      notifyError('Name and control-plane endpoint are required');
      return;
    }
    generating = true;
    generated = null;
    try {
      let clusterId: string | undefined;
      if (alsoInventory) {
        const created = (await client.post('/clusters', {
          name: name.trim(),
          controlPlaneVersion,
          talosVersion,
        })) as { id?: string };
        clusterId = created?.id;
      }
      const art = (await client.post('/clusters/generate-config', {
        name: name.trim(),
        endpoint: endpoint.trim(),
        talosVersion,
        kubernetesVersion: controlPlaneVersion,
        clusterId: clusterId || null,
      })) as {
        id: string;
        controlplaneConfig?: string;
        workerConfig?: string;
        hasSecrets?: boolean;
      };
      generated = art;
      success(
        art.hasSecrets
          ? 'Configs generated (secrets stored encrypted). Download below.'
          : 'Template configs generated (install talosctl on the TCS host for full secrets).'
      );
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Config generation failed');
    } finally {
      generating = false;
    }
  }

  function download(filename: string, body: string) {
    const blob = new Blob([body], { type: 'text/yaml' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
  }
</script>

<div class="create-page">
  <h1>Greenfield assist</h1>
  <p class="hint">
    TCS does not provision bare metal. Use this wizard to generate machine configs (via
    <code>talosctl gen config</code> when available on the TCS host, otherwise a template stub), then
    apply them out-of-band and <a href="/clusters/import">Import</a> the live cluster.
  </p>

  <form
    class="create-form"
    onsubmit={(e) => {
      e.preventDefault();
    }}
  >
    <div class="form-group">
      <label for="name">Cluster Name</label>
      <input id="name" type="text" bind:value={name} placeholder="my-cluster" required />
    </div>

    <div class="form-group">
      <label for="endpoint">Kubernetes API endpoint</label>
      <input
        id="endpoint"
        type="text"
        bind:value={endpoint}
        placeholder="https://controlplane.example:6443"
        required
      />
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
          <option value="v1.9.0">v1.9.0</option>
          <option value="v1.8.0">v1.8.0</option>
          <option value="v1.7.5">v1.7.5</option>
        </select>
      </div>
    </div>

    <label class="check">
      <input type="checkbox" bind:checked={alsoInventory} />
      Also create inventory record in TCS
    </label>

    <div class="form-actions">
      <Button variant="ghost" type="button" onclick={() => window.history.back()}>Cancel</Button>
      <Button variant="secondary" type="button" onclick={handleCreate} disabled={creating}>
        {creating ? 'Creating…' : 'Inventory only'}
      </Button>
      <Button variant="primary" type="button" onclick={generateConfigs} disabled={generating}>
        {generating ? 'Generating…' : 'Generate machine configs'}
      </Button>
    </div>
  </form>

  {#if generated}
    <section class="result">
      <h2>Generated artifact</h2>
      <p class="muted">id: <code>{generated.id}</code> · secrets encrypted: {generated.hasSecrets ? 'yes' : 'no'}</p>
      <div class="dl-row">
        {#if generated.controlplaneConfig}
          <Button
            variant="secondary"
            size="sm"
            onclick={() => download(`${name || 'cluster'}-controlplane.yaml`, generated!.controlplaneConfig!)}
          >
            Download controlplane.yaml
          </Button>
        {/if}
        {#if generated.workerConfig}
          <Button
            variant="secondary"
            size="sm"
            onclick={() => download(`${name || 'cluster'}-worker.yaml`, generated!.workerConfig!)}
          >
            Download worker.yaml
          </Button>
        {/if}
        <Button variant="primary" size="sm" onclick={() => goto('/clusters')}>Back to clusters</Button>
      </div>
      {#if generated.controlplaneConfig}
        <pre class="preview">{generated.controlplaneConfig.slice(0, 1200)}{#if generated.controlplaneConfig.length > 1200}…{/if}</pre>
      {/if}
    </section>
  {/if}
</div>

<style>
  .create-page h1 {
    margin: 0 0 0.75rem;
  }
  .hint {
    opacity: 0.9;
    margin-bottom: 1.25rem;
    max-width: 720px;
    line-height: 1.45;
  }
  .create-form {
    max-width: 640px;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1;
  }
  .form-row {
    display: flex;
    gap: 1rem;
  }
  .form-group label {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }
  .form-group input,
  .form-group select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.9rem;
  }
  .form-actions {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .result {
    margin-top: 2rem;
    max-width: 720px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1rem 1.25rem;
  }
  .result h2 {
    margin: 0 0 0.5rem;
    font-size: 1.05rem;
  }
  .muted {
    color: var(--tcs-text-muted);
    font-size: 0.85rem;
  }
  .dl-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0.75rem 0;
  }
  .preview {
    font-size: 0.75rem;
    overflow: auto;
    max-height: 280px;
    background: var(--tcs-background);
    padding: 0.75rem;
    border-radius: 6px;
  }
</style>
