<script lang="ts">
  import {
    deleteResource,
    scaleDeployment,
    cordonNode,
    uncordonNode,
    drainNode,
    applyManifest,
    type ApplyResult,
  } from '$lib/api/k8s';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';

  export interface Selection {
    kind: string; // 'pod' | 'deployment' | 'node' | ...
    name: string;
    ns?: string;
  }

  let {
    clusterId,
    selection,
    onMutated,
  }: {
    clusterId: string;
    selection: Selection | null;
    onMutated?: () => void;
  } = $props();

  let busy = $state(false);
  let replicas = $state(1);
  let showApply = $state(false);
  let manifest = $state('');
  let applyResults = $state<ApplyResult[] | null>(null);
  let drainInfo = $state<{ evicted: number; skipped: number; errors: number } | null>(null);

  const isDeployment = $derived(selection?.kind === 'deployments');
  const isNode = $derived(selection?.kind === 'nodes');
  const isPod = $derived(selection?.kind === 'pods');

  function target(): string {
    if (!selection) return '';
    return selection.ns ? `${selection.kind}/${selection.name} (${selection.ns})` : `${selection.kind}/${selection.name}`;
  }

  async function run(fn: () => Promise<unknown>, okMsg: string) {
    if (busy) return;
    busy = true;
    try {
      await fn();
      success(okMsg);
      onMutated?.();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Action failed');
    } finally {
      busy = false;
    }
  }

  function doDelete() {
    if (!selection) return;
    if (!confirm(`Delete ${target()}? This cannot be undone.`)) return;
    void run(
      () => deleteResource(clusterId, selection.kind, selection.name, selection.ns),
      `Deleted ${target()}`
    );
  }

  function doScale() {
    if (!selection) return;
    void run(
      () => scaleDeployment(clusterId, selection.ns ?? 'default', selection.name, replicas),
      `Scaled ${selection.name} to ${replicas}`
    );
  }

  function doCordon(cordon: boolean) {
    if (!selection) return;
    const fn = cordon ? cordonNode : uncordonNode;
    void run(() => fn(clusterId, selection.name), `${cordon ? 'Cordoned' : 'Uncordoned'} ${selection.name}`);
  }

  function doDrain() {
    if (!selection) return;
    if (!confirm(`Drain node ${selection.name}?\n\nIts non-DaemonSet pods will be evicted.`)) return;
    drainInfo = null;
    void run(async () => {
      const r = await drainNode(clusterId, selection.name, false);
      drainInfo = { evicted: r.evicted.length, skipped: r.skipped.length, errors: r.errors.length };
      if (r.errors.length) notifyError(`Drain finished with ${r.errors.length} error(s)`);
    }, `Drained ${selection.name}`);
  }

  async function doApply() {
    if (!manifest.trim()) return;
    busy = true;
    applyResults = null;
    try {
      const r = await applyManifest(clusterId, manifest);
      applyResults = r.results;
      const ok = r.results.filter((x) => x.status === 'applied').length;
      if (ok === r.results.length) success(`Applied ${ok} document(s)`);
      else notifyError(`Applied ${ok}/${r.results.length} document(s)`);
      onMutated?.();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Apply failed');
    } finally {
      busy = false;
    }
  }
</script>

<div class="actions">
  {#if selection}
    <div class="sel">
      <span class="sel-label">Selected</span>
      <code class="sel-target">{target()}</code>
    </div>

    <div class="btn-row">
      {#if isDeployment}
        <div class="scale-group">
          <input type="number" min="0" title="Desired replica count for this deployment" bind:value={replicas} />
          <Button variant="secondary" size="sm" title="Scale this deployment to the replica count above" onclick={doScale} disabled={busy}>Scale</Button>
        </div>
      {/if}
      {#if isNode}
        <Button variant="secondary" size="sm" title="Cordon this node so no new pods are scheduled on it" onclick={() => doCordon(true)} disabled={busy}>Cordon</Button>
        <Button variant="secondary" size="sm" title="Uncordon this node to allow new pod scheduling" onclick={() => doCordon(false)} disabled={busy}>Uncordon</Button>
        <Button variant="danger" size="sm" title="Evict non-DaemonSet pods from this node" onclick={doDrain} disabled={busy}>Drain</Button>
      {/if}
      {#if isPod || isDeployment}
        <Button variant="danger" size="sm" title="Delete this resource (cannot be undone)" onclick={doDelete} disabled={busy}>Delete</Button>
      {/if}
      <Button variant="ghost" size="sm" title="Show/hide the YAML manifest apply panel" onclick={() => (showApply = !showApply)} disabled={busy}>
        {showApply ? 'Hide Apply' : 'Apply YAML'}
      </Button>
    </div>

    {#if drainInfo}
      <div class="result">
        Drain: {drainInfo.evicted} evicted · {drainInfo.skipped} skipped · {drainInfo.errors} error(s)
      </div>
    {/if}

    {#if showApply}
      <div class="apply">
        <textarea
          title="Kubernetes manifest(s) to server-side apply; separate multiple documents with ---"
          bind:value={manifest}
          rows="8"
          placeholder="apiVersion: apps/v1&#10;kind: Deployment&#10;metadata:&#10;  name: …&#10;spec: …"
        ></textarea>
        <div class="apply-foot">
          <span class="hint">Server-side apply. Multiple documents separated by `---`.</span>
          <Button variant="primary" size="sm" title="Server-side apply the manifest(s) above" onclick={doApply} disabled={busy || !manifest.trim()}>
            {busy ? 'Applying…' : 'Apply'}
          </Button>
        </div>
        {#if applyResults}
          <ul class="apply-results">
            {#each applyResults as r (r.kind + r.name + r.namespace)}
              <li class:ok={r.status === 'applied'}>
                <span class="mono">{r.kind}/{r.name}</span> {r.namespace && `(${r.namespace})`} — {r.status}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  {:else}
    <p class="hint">Select a resource to enable actions.</p>
  {/if}
</div>

<style>
  .actions {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.75rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
  }
  .sel {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .sel-label {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tcs-text-muted);
  }
  .sel-target {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    color: var(--tcs-text);
  }
  .btn-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .scale-group {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .scale-group input {
    width: 3.5rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text);
    padding: 0.3rem 0.4rem;
    font-size: 0.85rem;
  }
  .result {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }
  .apply {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .apply textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem;
    color: var(--tcs-text);
    resize: vertical;
  }
  .apply-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .hint {
    color: var(--tcs-text-muted);
    font-size: 0.78rem;
  }
  .apply-results {
    margin: 0;
    padding-left: 1rem;
    font-size: 0.8rem;
    list-style: none;
  }
  .apply-results li {
    margin-bottom: 0.2rem;
  }
  .apply-results li.ok {
    color: var(--tcs-success);
  }
  .apply-results li:not(.ok) {
    color: var(--tcs-error, #ef4444);
  }
  .mono {
    font-family: ui-monospace, monospace;
  }
</style>
