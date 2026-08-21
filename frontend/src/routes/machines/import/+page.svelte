<script lang="ts">
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { goto } from '$app/navigation';
  import Button from '$lib/components/Button.svelte';

  let format = $state<'yaml' | 'csv'>('yaml');
  let content = $state(`# Example inventory
cluster:
  name: rack-a
  talosVersion: v1.13.7
  kubernetesVersion: v1.36.3
machines:
  - hostname: cp-1
    role: controlplane
    mac: aa:bb:cc:dd:ee:01
    bmc:
      address: 10.90.0.11
      username: root
      password: change-me
      type: auto
  - hostname: w-1
    role: worker
    mac: aa:bb:cc:dd:ee:02
    bmc:
      address: 10.90.0.12
      username: root
      password: change-me
`);
  let createCluster = $state(true);
  let createClusterName = $state('');
  let clusterId = $state('');
  let clusters = $state<Array<{ id: string; name: string }>>([]);
  let preview = $state<any>(null);
  let result = $state<any>(null);
  let busy = $state(false);

  async function loadClusters() {
    try {
      clusters = ((await client.get('/clusters')) as any[]) || [];
    } catch {
      clusters = [];
    }
  }
  loadClusters();

  async function onFile(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    content = await file.text();
    if (file.name.endsWith('.csv')) format = 'csv';
    else format = 'yaml';
  }

  async function doPreview() {
    busy = true;
    result = null;
    try {
      preview = await client.post('/machines/import/preview', {
        format,
        content,
      });
      success(`Preview: ${preview.machines?.length || 0} rows`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Preview failed');
    } finally {
      busy = false;
    }
  }

  async function doImport() {
    busy = true;
    try {
      result = await client.post('/machines/import', {
        format,
        content,
        createCluster: createCluster && !clusterId,
        createClusterName: createClusterName || undefined,
        clusterId: clusterId || undefined,
        upsertByMac: true,
      });
      success(
        `Imported: ${result.created} created, ${result.updated} updated` +
          (result.errors?.length ? `, ${result.errors.length} row errors` : '')
      );
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Import failed');
    } finally {
      busy = false;
    }
  }
</script>

<div class="import-page">
  <div class="page-header">
    <h1>Import machine inventory</h1>
    <a href="/"><Button variant="ghost" size="sm">Back to Dashboard</Button></a>
  </div>
  <p class="hint">
    Bulk-add servers for PXE provisioning (MAC + BMC) or address-based assisted install.
    YAML is canonical; CSV uses a flat header row.
  </p>

  <div class="row">
    <label>
      Format
      <select bind:value={format}>
        <option value="yaml">YAML</option>
        <option value="csv">CSV</option>
      </select>
    </label>
    <label>
      File
      <input type="file" accept=".yaml,.yml,.csv,text/*" onchange={onFile} />
    </label>
  </div>

  <textarea bind:value={content} rows="16" spellcheck="false"></textarea>

  <div class="row">
    <label class="check">
      <input type="checkbox" bind:checked={createCluster} disabled={!!clusterId} />
      Create cluster from YAML name (if no cluster selected)
    </label>
    <label>
      Cluster name override
      <input type="text" bind:value={createClusterName} placeholder="optional" />
    </label>
    <label>
      Attach to existing cluster
      <select bind:value={clusterId}>
        <option value="">— none / create —</option>
        {#each clusters as c (c.id)}
          <option value={c.id}>{c.name}</option>
        {/each}
      </select>
    </label>
  </div>

  <div class="actions">
    <Button variant="secondary" onclick={doPreview} disabled={busy}>Preview</Button>
    <Button variant="primary" onclick={doImport} disabled={busy}>Import</Button>
  </div>

  {#if preview}
    <section class="card">
      <h2>Preview</h2>
      {#if preview.errors?.length}
        <ul class="errs">
          {#each preview.errors as e}
            <li>Row {e.index}: {e.message}</li>
          {/each}
        </ul>
      {/if}
      <table class="data-table">
        <thead>
          <tr>
            <th>#</th>
            <th>Hostname</th>
            <th>Role</th>
            <th>MAC</th>
            <th>Address</th>
            <th>BMC</th>
          </tr>
        </thead>
        <tbody>
          {#each preview.machines || [] as m}
            <tr>
              <td>{m.index}</td>
              <td>{m.hostname || '—'}</td>
              <td>{m.role}</td>
              <td class="mono">{m.mac || '—'}</td>
              <td class="mono">{m.address || '—'}</td>
              <td class="mono">{m.bmcAddress || '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  {#if result}
    <section class="card">
      <h2>Result</h2>
      <p>
        Created <strong>{result.created}</strong>, updated <strong>{result.updated}</strong>
        {#if result.clusterId}
          · cluster <code>{result.clusterId}</code>
        {/if}
      </p>
      {#if result.errors?.length}
        <ul class="errs">
          {#each result.errors as e}
            <li>Row {e.index}: {e.message}</li>
          {/each}
        </ul>
      {/if}
      {#if result.clusterId}
        <Button
          variant="primary"
          size="sm"
          onclick={() => goto(`/clusters/${result.clusterId}`)}
        >
          Open cluster
        </Button>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .hint { color: var(--tcs-text-muted); max-width: 48rem; }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin: 1rem 0;
    align-items: end;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.85rem;
  }
  label.check { flex-direction: row; align-items: center; gap: 0.5rem; }
  textarea, input, select {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
  }
  textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    min-height: 16rem;
  }
  .actions { display: flex; gap: 0.5rem; margin: 1rem 0; }
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem;
    margin-top: 1rem;
  }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.35rem 0.4rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .errs { color: var(--tcs-error); font-size: 0.85rem; }
</style>
