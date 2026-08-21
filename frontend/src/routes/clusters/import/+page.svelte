<script lang="ts">
  import { onMount } from 'svelte';
  import { previewImport, importCluster, loading as storeLoading, error as storeError, loadClusters } from '$lib/stores/clusters';
  import type { DiscoveredCluster, ImportResult } from '$lib/stores/clusters';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Spinner from '$lib/components/Spinner.svelte';
  import Button from '$lib/components/Button.svelte';

  let name = $state('');
  let kubeconfig = $state('');
  let talosconfig = $state('');
  let step = $state<'input' | 'preview' | 'importing' | 'done'>('input');
  let preview = $state<DiscoveredCluster | null>(null);
  let err = $state<string | null>(null);
  let isTalos = $state(false);
  let importResult = $state<ImportResult | null>(null);

  onMount(() => {
    if (!kubeconfig) {
      const stored = localStorage.getItem('last-kubeconfig');
      if (stored) {
        kubeconfig = stored;
      }
    }
  });

  async function handlePreview() {
    if (!kubeconfig.trim()) {
      err = 'Please paste a kubeconfig';
      return;
    }
    err = null;
    storeLoading.set(true);
    try {
      preview = await previewImport(kubeconfig, name);
      isTalos = preview.isTalos;
      step = 'preview';
      if (preview.name && !name) {
        name = preview.name;
      }
    } catch (e: unknown) {
      err = e instanceof Error ? e.message : 'Failed to preview import';
    } finally {
      storeLoading.set(false);
    }
  }

  async function handleImport() {
    if (!name.trim()) {
      err = 'Please provide a cluster name';
      return;
    }
    step = 'importing';
    storeLoading.set(true);
    err = null;
    try {
      importResult = await importCluster(name, kubeconfig, talosconfig);
      step = 'done';
      const n = importResult.machinesImported ?? 0;
      success(`Cluster "${name}" imported with ${n} machines`);
      localStorage.setItem('last-kubeconfig', kubeconfig);
    } catch (e: unknown) {
      err = e instanceof Error ? e.message : 'Failed to import cluster';
      step = 'preview';
    } finally {
      storeLoading.set(false);
    }
  }

  function nodeCount() {
    if (!preview) return 0;
    return preview.controlPlaneNodes.length + preview.workerNodes.length;
  }
</script>

<div class="import-page">
  <div class="page-header">
    <div>
      <h1>Import Cluster</h1>
      <p class="subtitle">Import an existing Talos Linux cluster using its kubeconfig</p>
    </div>
    <a href="/">
      <Button variant="ghost">Back to Dashboard</Button>
    </a>
  </div>

  {#if $storeError}
    <div class="error-banner">{$storeError}</div>
  {/if}

  <!-- Step 1: Input -->
  {#if step === 'input'}
    <div class="form-card">
      <h2>Kubeconfig</h2>
      <p class="hint">Paste the contents of your kubeconfig file (usually ~/.kube/config)</p>

      <div class="form-group">
        <label for="cluster-name">Cluster Name</label>
        <input
          id="cluster-name"
          type="text"
          bind:value={name}
          placeholder="my-talos-cluster"
          class="input"
        />
      </div>

      <div class="form-group">
        <label for="kubeconfig">Kubeconfig YAML</label>
        <textarea
          id="kubeconfig"
          bind:value={kubeconfig}
          placeholder="apiVersion: v1
kind: Config
clusters:
- cluster:
    server: https://..."
          rows={16}
          class="input kubeconfig-input"
        ></textarea>
      </div>

      <div class="form-group">
        <label for="talosconfig">Talosconfig YAML (optional, recommended)</label>
        <p class="hint">
          Paste <code>~/.talos/config</code> so TCS can call the Talos API (etcd backups, config apply, reboot).
          Without it, import still works for inventory only.
        </p>
        <textarea
          id="talosconfig"
          bind:value={talosconfig}
          placeholder="context: my-cluster
contexts:
  my-cluster:
    endpoints:
      - https://10.0.0.2:50000
    ca: ...
    crt: ...
    key: ..."
          rows={10}
          class="input kubeconfig-input"
        ></textarea>
      </div>

      {#if err}
        <div class="error">{err}</div>
      {/if}

      <div class="form-actions">
        <a href="/">
          <Button variant="ghost">Cancel</Button>
        </a>
        <Button variant="primary" disabled={$storeLoading} onclick={handlePreview}>
          {$storeLoading ? 'Validating...' : 'Validate & Preview'}
        </Button>
      </div>
    </div>
  {:else if step === 'preview'}
    <!-- Step 2: Preview -->
    <div class="form-card">
      <div class="preview-header">
        <h2>Discovered Cluster</h2>
        <Button variant="ghost" onclick={() => step = 'input'}>← Edit</Button>
      </div>

      {#if preview}
        <div class="discovery-result">
          <div class="result-row">
            <span class="label">Cluster</span>
            <span class="value">{preview.name}</span>
          </div>
          <div class="result-row">
            <span class="label">API Server</span>
            <span class="value">{preview.server}</span>
          </div>
          <div class="result-row">
            <span class="label">Kubernetes</span>
            <span class="value">{preview.kubernetesVersion}</span>
          </div>
          <div class="result-row">
            <span class="label">Talos Version</span>
            <span class="value">{preview.talosVersion || 'Unknown'}</span>
          </div>
          <div class="result-row">
            <span class="label">OS</span>
            <span class="value">
              <span class={isTalos ? 'badge talos' : 'badge non-talos'}>
                {isTalos ? 'Talos Linux ✓' : 'Not Talos'}
              </span>
            </span>
          </div>
          <div class="result-row">
            <span class="label">Nodes</span>
            <span class="value">
              {preview.controlPlaneNodes.length} control-plane,
              {preview.workerNodes.length} worker ({nodeCount()} total)
            </span>
          </div>
        </div>

        <!-- Node details -->
        {#if preview.controlPlaneNodes.length > 0 || preview.workerNodes.length > 0}
          <h3>Nodes</h3>
          <table class="data-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>IP</th>
                <th>Role</th>
                <th>K8s</th>
                <th>Talos</th>
                <th>OS</th>
              </tr>
            </thead>
            <tbody>
              {#each preview.controlPlaneNodes as node}
                <tr>
                  <td>{node.name}</td>
                  <td>{node.internalIp || '—'}</td>
                  <td><span class="role-tag control-plane">control-plane</span></td>
                  <td>{node.kubernetesVersion}</td>
                  <td>{node.talosVersion || '—'}</td>
                  <td>{node.osImage}</td>
                </tr>
              {/each}
              {#each preview.workerNodes as node}
                <tr>
                  <td>{node.name}</td>
                  <td>{node.internalIp || '—'}</td>
                  <td><span class="role-tag worker">worker</span></td>
                  <td>{node.kubernetesVersion}</td>
                  <td>{node.talosVersion || '—'}</td>
                  <td>{node.osImage}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}

        {#if !isTalos}
          <div class="warning">
            <strong>Warning:</strong> This cluster does not appear to be running Talos Linux.
            You can still import it, but Talos-specific features (config patches, upgrades, backups)
            will not be available.
          </div>
        {/if}

        {#if err}
          <div class="error">{err}</div>
        {/if}

        <div class="form-actions">
          <a href="/">
            <Button variant="ghost">Cancel</Button>
          </a>
          <Button
            variant="primary"
            disabled={$storeLoading}
            onclick={handleImport}
          >
            {$storeLoading ? 'Importing...' : `Import "${name}" (${nodeCount()} nodes)`}
          </Button>
        </div>
      {/if}
    </div>
  {:else if step === 'importing'}
    <div class="form-card center">
      <Spinner />
      <h2>Importing Cluster...</h2>
      <p class="hint">Creating cluster record and discovering machines</p>
    </div>
  {:else if step === 'done'}
    <div class="form-card center">
      <div class="success-icon">✓</div>
      <h2>Cluster Imported</h2>
      {#if importResult}
        <p class="success-details">
          <strong>{importResult.cluster.name}</strong> imported with
          <strong>{importResult.machinesImported}</strong> machines.
        </p>
      {/if}
      {#if err}
        <div class="error">{err}</div>
      {/if}
      <div class="form-actions">
        <a href="/">
          <Button variant="ghost">View All Clusters</Button>
        </a>
        {#if importResult}
          <a href="/clusters/{importResult.cluster.id}">
            <Button variant="primary">View Cluster</Button>
          </a>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .import-page h1 { margin: 0; }
  .subtitle { color: var(--tcs-text-muted); margin-top: 0.25rem; font-size: 0.9rem; }
  .page-header { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 1.5rem; gap: 1rem; }
  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    color: var(--tcs-error);
    margin-bottom: 1rem;
  }

  .form-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 12px;
    padding: 1.5rem;
  }
  .form-card h2 { margin-top: 0; }
  .form-card.center { text-align: center; }

  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; }

  .form-group { margin-bottom: 1rem; }
  .form-group label {
    display: block;
    font-size: 0.85rem;
    font-weight: 600;
    margin-bottom: 0.4rem;
    color: var(--tcs-text-muted);
  }
  .input {
    width: 100%;
    padding: 0.6rem 0.8rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    color: var(--tcs-text);
    font-size: 0.9rem;
    font-family: inherit;
  }
  .input:focus {
    outline: none;
    border-color: var(--tcs-primary);
  }
  .kubeconfig-input {
    font-family: monospace;
    font-size: 0.8rem;
    line-height: 1.5;
  }

  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-error);
    font-size: 0.85rem;
    margin-top: 0.75rem;
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.5rem;
  }

  .preview-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }
  .preview-header h2 { margin: 0; }

  .discovery-result {
    background: var(--tcs-background);
    border-radius: 8px;
    padding: 1rem;
    margin-bottom: 1rem;
  }
  .result-row {
    display: flex;
    justify-content: space-between;
    padding: 0.4rem 0;
    border-bottom: 1px solid var(--tcs-border);
  }
  .result-row:last-child { border-bottom: none; }
  .result-row .label { color: var(--tcs-text-muted); font-size: 0.85rem; }
  .result-row .value { font-weight: 500; }

  .badge {
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
  }
  .badge.talos { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .badge.non-talos { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }

  .warning {
    background: rgba(245, 158, 11, 0.1);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 6px;
    padding: 0.75rem 1rem;
    color: var(--tcs-warning);
    font-size: 0.85rem;
    margin-top: 1rem;
  }

  .data-table { width: 100%; border-collapse: collapse; margin-top: 0.5rem; font-size: 0.85rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .data-table tr:hover { background: var(--tcs-surface-hover); }

  .role-tag {
    font-size: 0.7rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
  }
  .role-tag.control-plane { background: rgba(99, 102, 241, 0.2); color: #818cf8; }
  .role-tag.worker { background: rgba(16, 185, 129, 0.2); color: #34d399; }

  .success-icon {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: rgba(16, 185, 129, 0.2);
    color: var(--tcs-success);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.5rem;
    margin: 0 auto 1rem;
  }
  .success-details { color: var(--tcs-text-muted); }
</style>
