<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface ConfigPatch {
    id: string;
    path: string;
    value: string;
    priority: number;
    machineId: string | null;
    scope: 'cluster' | 'machine';
    createdAt: string;
  }
  
  let patches = $state<ConfigPatch[]>([]);
  let loading = $state(true);
  let error = $state('');
  let showEditor = $state(false);
  let newPatch = $state({ path: '', value: '', priority: 0 });
  let saving = $state(false);
  
  onMount(async () => {
    try {
      patches = await client.get(`/clusters/${$page.params.id}/config`) as ConfigPatch[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load config patches';
    } finally {
      loading = false;
    }
  });
  
  async function addPatch() {
    if (!newPatch.path.trim()) return;
    saving = true;
    try {
      await client.post(`/clusters/${$page.params.id}/config`, newPatch);
      showEditor = false;
      newPatch = { path: '', value: '', priority: 0 };
      success('Config patch added');
      await loadPatches();
    } catch {
      notifyError('Failed to add patch');
    } finally {
      saving = false;
    }
  }
  
  async function deletePatch(id: string) {
    try {
      await client.delete(`/clusters/${$page.params.id}/config/${id}`);
      patches = patches.filter(p => p.id !== id);
      success('Patch removed');
    } catch {
      notifyError('Failed to remove patch');
    }
  }
  
  async function loadPatches() {
    loading = true;
    try {
      patches = await client.get(`/clusters/${$page.params.id}/config`) as ConfigPatch[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load config patches';
    } finally {
      loading = false;
    }
  }

  let applying = $state(false);
  let lastApply = $state<{
    dryRun?: boolean;
    count?: number;
    appliedTo?: string[];
    errors?: string[];
    documents?: Array<{ address?: string; patchPreview?: string }>;
  } | null>(null);

  async function applyAll(dryRun = false) {
    applying = true;
    lastApply = null;
    try {
      const res = (await client.post(`/clusters/${$page.params.id}/config/apply`, {
        dryRun,
      })) as {
        count: number;
        appliedTo?: string[];
        dryRun?: boolean;
        errors?: string[];
        documents?: Array<{ address?: string; patchPreview?: string }>;
      };
      lastApply = res;
      const errN = res.errors?.length ?? 0;
      if (errN > 0) {
        notifyError(
          dryRun
            ? `Dry-run: ${res.count} ok, ${errN} error(s)`
            : `Applied ${res.count}; ${errN} error(s)`
        );
      } else {
        success(
          dryRun
            ? `Dry-run OK for ${res.count} machine(s)`
            : `Applied patches to ${res.count} machine(s)`
        );
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to apply patches via Talos API');
    } finally {
      applying = false;
    }
  }
</script>

<div class="config-page">
  <div class="page-header">
    <h1>Config Patches</h1>
    <div class="header-actions" style="display:flex;gap:0.5rem;">
      <Button variant="ghost" onclick={() => applyAll(true)} disabled={applying || patches.length === 0}>
        Dry-run
      </Button>
      <Button variant="secondary" onclick={() => applyAll(false)} disabled={applying || patches.length === 0}>
        {applying ? 'Applying…' : 'Apply to cluster'}
      </Button>
      <Button variant="primary" onclick={() => showEditor = !showEditor}>
        {showEditor ? 'Cancel' : 'Add Patch'}
      </Button>
    </div>
  </div>
  <p class="hint" style="opacity:0.8;margin-bottom:1rem;">
    Patches are stored in TCS, merged into each node’s live machine config (COSI), then applied with
    Talos <code>ApplyConfiguration</code> (no-reboot). Falls back to <code>talosctl patch mc</code> if needed.
    Requires a talosconfig on the cluster and reachability to node :50000.
  </p>

  {#if lastApply}
    <div class="apply-result">
      <h3>{lastApply.dryRun ? 'Dry-run result' : 'Apply result'}</h3>
      <p>
        {lastApply.count ?? 0} machine(s)
        {#if lastApply.errors?.length}
          · {lastApply.errors.length} error(s)
        {/if}
      </p>
      {#if lastApply.documents?.[0]?.patchPreview}
        <details>
          <summary>Patch preview</summary>
          <pre>{lastApply.documents[0].patchPreview}</pre>
        </details>
      {/if}
      {#if lastApply.errors?.length}
        <details open>
          <summary>Errors</summary>
          <ul>
            {#each lastApply.errors as err}
              <li class="err-line">{err}</li>
            {/each}
          </ul>
        </details>
      {/if}
      {#if lastApply.appliedTo?.length}
        <details>
          <summary>Applied to</summary>
          <ul>
            {#each lastApply.appliedTo as line}
              <li>{line}</li>
            {/each}
          </ul>
        </details>
      {/if}
    </div>
  {/if}
  
  {#if showEditor}
    <div class="patch-editor">
      <div class="form-row">
        <div class="form-group">
          <label for="path">Document Path</label>
          <input
            id="path"
            type="text"
            bind:value={newPatch.path}
            placeholder="/machine/sysctls/net.ipv4.ip_forward"
          />
        </div>
        <div class="form-group" style="max-width: 80px;">
          <label for="priority">Priority</label>
          <input
            id="priority"
            type="number"
            bind:value={newPatch.priority}
          />
        </div>
      </div>
      <div class="form-group">
        <label for="value">Value (YAML)</label>
        <textarea
          id="value"
          bind:value={newPatch.value}
          rows="6"
          placeholder="true"
        ></textarea>
      </div>
      <div class="editor-actions">
        <Button variant="primary" onclick={addPatch} disabled={saving}>
          {saving ? 'Saving...' : 'Apply Patch'}
        </Button>
      </div>
    </div>
  {/if}
  
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if patches.length === 0}
    <div class="empty-state">
      <p>No config patches for this cluster</p>
      <p class="hint">Add patches to extend the default Talos configuration.</p>
    </div>
  {:else}
    <div class="patches-list">
      {#each patches as patch (patch.id)}
        <div class="patch-card">
          <div class="patch-header">
            <code class="patch-path">{patch.path}</code>
            <span class="scope-badge {patch.scope}">{patch.scope}</span>
            <span class="priority">Priority: {patch.priority}</span>
            <Button variant="danger" size="sm" onclick={() => deletePatch(patch.id)}>Remove</Button>
          </div>
          <pre class="patch-value"><code>{patch.value}</code></pre>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .config-page h1 { margin: 0; }
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

  .apply-result {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin-bottom: 1.5rem;
  }
  .apply-result h3 { margin: 0 0 0.5rem; font-size: 1rem; }
  .apply-result pre {
    background: var(--tcs-background);
    padding: 0.75rem;
    border-radius: 6px;
    overflow: auto;
    font-size: 0.8rem;
  }
  .apply-result ul { margin: 0.5rem 0 0; padding-left: 1.25rem; }
  .apply-result .err-line { color: var(--tcs-error, #f87171); font-size: 0.85rem; word-break: break-word; }
  
  .patch-editor {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }
  .form-row { display: flex; gap: 1rem; margin-bottom: 1rem; }
  .form-group { display: flex; flex-direction: column; gap: 0.4rem; flex: 1; }
  .form-group label { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .form-group input, .form-group textarea {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    outline: none;
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 0.875rem;
  }
  .form-group input:focus, .form-group textarea:focus {
    border-color: var(--tcs-primary);
  }
  .form-group textarea { resize: vertical; }
  .editor-actions { display: flex; justify-content: flex-end; margin-top: 1rem; }
  
  .patches-list { display: flex; flex-direction: column; gap: 0.75rem; }
  .patch-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .patch-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .patch-path {
    font-size: 0.8rem;
    color: var(--tcs-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .scope-badge {
    font-size: 0.65rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    text-transform: uppercase;
  }
  .scope-badge.cluster { background: rgba(79, 139, 255, 0.15); color: var(--tcs-secondary); }
  .scope-badge.machine { background: rgba(245, 158, 11, 0.15); color: var(--tcs-warning); }
  .priority {
    font-size: 0.75rem;
    color: var(--tcs-text-muted);
    margin-left: auto;
  }
  .patch-value {
    margin: 0;
    padding: 1rem;
    font-size: 0.8rem;
    line-height: 1.6;
    overflow-x: auto;
    color: var(--tcs-text-muted);
  }
</style>
