<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { onMount, onDestroy } from 'svelte';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import { client } from '$lib/api/client';
  import {
    convertPreview,
    convertStart,
    convertStatus,
    convertCancel,
    fetchFactoryExtensions,
    type ConvertPreview,
    type ConvertStatus,
    type ConvertNode,
    type FactoryExtensionItem,
  } from '$lib/api/convert';
  import { formatBytes } from '$lib/api/types';

  const cid = $page.params.id ?? '';

  type Step = 'setup' | 'preview' | 'confirm' | 'monitor' | 'done';
  let step = $state<Step>('setup');

  // ── global ─────────────────────────────────────────────────────────
  let busy = $state(false);
  let error = $state('');

  // ── setup ──────────────────────────────────────────────────────────
  let clusterName = $state('');
  let talosVersion = $state('v1.13.7');
  let knownVersions = $state<string[]>(['v1.13.7', 'v1.13.6', 'v1.13.5', 'v1.12.10', 'v1.12.9', 'v1.12.8']);
  let clusterTalos = $state('');
  let extensions = $state<FactoryExtensionItem[]>([]);
  let selectedModules = $state<Set<string>>(new Set());
  let factoryBusy = $state(false);
  let factoryError = $state('');

  function shortName(full: string): string {
    const i = full.indexOf('/');
    return i >= 0 ? full.slice(i + 1) : full;
  }

  function toggleModule(name: string) {
    const next = new Set(selectedModules);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selectedModules = next;
  }

  async function loadClusterMeta() {
    try {
      const c = (await client.get(`/clusters/${cid}`)) as {
        name?: string;
        talosVersion?: string;
        talos_version?: string;
      };
      clusterName = c.name || '';
      clusterTalos = c.talosVersion || c.talos_version || '';
    } catch {
      // non-fatal — the wizard still works with the default version
    }
  }

  async function loadExtensions(version: string) {
    factoryBusy = true;
    factoryError = '';
    try {
      extensions = await fetchFactoryExtensions(version);
    } catch (e: unknown) {
      factoryError = e instanceof Error ? e.message : 'Failed to load module catalog';
      extensions = [];
    } finally {
      factoryBusy = false;
    }
  }

  // Re-load the module catalog whenever the selected Talos version changes.
  $effect(() => {
    const v = talosVersion;
    if (v && step === 'setup') loadExtensions(v);
  });

  async function runPreview() {
    busy = true;
    error = '';
    try {
      const res = await convertPreview(cid, {
        talosVersion,
        modules: [...selectedModules],
      });
      preview = res;
      clusterName = res.clusterName || clusterName;
      step = 'preview';
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : 'Failed to analyze cluster';
      error = msg;
      notifyError(msg);
    } finally {
      busy = false;
    }
  }

  // ── preview ────────────────────────────────────────────────────────
  let preview = $state<ConvertPreview | null>(null);
  let usedRecommended = $state(false);

  function summarizeNetwork(n: ConvertNode): string {
    const net = n.network;
    const bits: string[] = [];
    for (const b of net.bonds) bits.push(`${b.name}(${b.mode})`);
    for (const v of net.vlans) bits.push(`vlan ${v.id}`);
    const iface = net.interfaces[0];
    if (iface) {
      bits.push(`${iface.ip}/${iface.cidr}`);
      if (net.gateway) bits.push(`gw ${net.gateway}`);
      if (iface.mtu && iface.mtu !== 1500) bits.push(`mtu ${iface.mtu}`);
    } else if (net.gateway) {
      bits.push(`gw ${net.gateway}`);
    }
    if (net.ovsBridges?.length) bits.push(`OVS: ${net.ovsBridges.join(',')}`);
    return bits.length ? bits.join(' · ') : '—';
  }

  function useRecommended() {
    if (!preview) return;
    const next = new Set(selectedModules);
    for (const node of preview.nodes) {
      for (const m of node.recommendedModules) next.add(m);
    }
    selectedModules = next;
    usedRecommended = true;
    success('Applied the recommended module set');
  }

  const cpCount = $derived((preview?.nodes || []).filter((n) => n.role === 'control-plane').length);
  const workerCount = $derived((preview?.nodes || []).filter((n) => n.role === 'worker').length);
  const canContinue = $derived(!!preview && preview.canConvert && preview.blockers.length === 0);

  // ── confirm ────────────────────────────────────────────────────────
  function orderNodes(): { name: string; role: string }[] {
    if (!preview) return [];
    return [...preview.nodes].sort((a, b) => {
      if (a.role === 'control-plane' && b.role !== 'control-plane') return -1;
      if (b.role === 'control-plane' && a.role !== 'control-plane') return 1;
      return a.name.localeCompare(b.name);
    });
  }

  async function startConversion() {
    const n = preview?.nodes.length ?? 0;
    const ok = confirm(
      `Start in-place conversion of ${n} node(s) to Talos ${talosVersion}?\n\n` +
        'An etcd snapshot is taken first. During control-plane conversion the API ' +
        'server is briefly unavailable. Workers are drained before conversion. ' +
        'This cannot be undone per-node.'
    );
    if (!ok) return;
    busy = true;
    error = '';
    try {
      const res = await convertStart(cid, {
        talosVersion,
        modules: [...selectedModules],
        nodes: orderNodes(),
      });
      jobId = res.jobId;
      step = 'monitor';
      success(`Conversion started: ${res.jobId}`);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : 'Failed to start conversion';
      error = msg;
      notifyError(msg);
    } finally {
      busy = false;
    }
  }

  // ── monitor ────────────────────────────────────────────────────────
  let jobId = $state('');
  let status = $state<ConvertStatus | null>(null);
  let statusError = $state('');
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  async function pollOnce() {
    try {
      const s = await convertStatus(cid);
      status = s;
      statusError = '';
    } catch (e: unknown) {
      // 404/{"error":...} when no active job — surface it but keep polling.
      statusError = e instanceof Error ? e.message : 'No conversion job found';
    }
  }

  // While on the monitor step, poll every 3s; clear the timer on exit/unmount.
  $effect(() => {
    if (step !== 'monitor') {
      if (pollTimer) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
      return;
    }
    void pollOnce();
    pollTimer = setInterval(() => void pollOnce(), 3000);
    return () => {
      if (pollTimer) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    };
  });

  async function cancelConversion() {
    if (!confirm('Cancel the running conversion job?')) return;
    busy = true;
    error = '';
    try {
      await convertCancel(cid);
      success('Cancel requested');
      await pollOnce();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : 'Cancel failed';
      error = msg;
      notifyError(msg);
    } finally {
      busy = false;
    }
  }

  function toDone() {
    step = 'done';
  }

  const failedNodeCount = $derived((status?.nodes || []).filter((n) => n.status === 'failed').length);
  const doneNodeCount = $derived((status?.nodes || []).filter((n) => n.status === 'done').length);

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
  });

  onMount(() => {
    void loadClusterMeta();
  });
</script>

<svelte:head>
  <title>Convert to Talos</title>
</svelte:head>

<div class="convert-wizard">
  {#if error}
    <div class="error-banner">
      {error}
      <button class="banner-dismiss" title="Dismiss" onclick={() => (error = '')}>×</button>
    </div>
  {/if}

  <!-- ── Step 1: setup ─────────────────────────────────────────────── -->
  {#if step === 'setup'}
    <div class="header-row">
      <div>
        <h1>Convert to Talos</h1>
        <p class="subtitle">
          In-place conversion of this cluster's nodes to Talos Linux. Cluster state (etcd) is
          snapshotted and restored; distributed persistent volumes are unaffected.
        </p>
      </div>
    </div>

    <div class="panel">
      <div class="setup-grid">
        <label>
          Talos version
          <select title="Target Talos version for the installer image" bind:value={talosVersion}>
            {#if clusterTalos && clusterTalos !== talosVersion}
              <option value={clusterTalos}>{clusterTalos} (current)</option>
            {/if}
            {#each knownVersions as v (v)}
              <option value={v}>{v}{clusterTalos === v ? ' (current)' : ''}</option>
            {/each}
          </select>
        </label>
      </div>
      <p class="hint">Target Talos version for the installer image.</p>

      {#if factoryError}
        <p class="hint error">{factoryError}</p>
      {:else if factoryBusy}
        <p class="hint">Loading module catalog…</p>
      {:else}
        <p class="hint">
          Image Factory modules baked into the installer. Select drivers your nodes need.
        </p>
        <div class="module-picker">
          {#each extensions as f (f.name)}
            <label class="module-option" title={f.description || f.ref || ''}>
              <input
                type="checkbox"
                checked={selectedModules.has(f.name)}
                onchange={() => toggleModule(f.name)}
              />
              <span class="module-name mono">{shortName(f.name)}</span>
              {#if f.author}<span class="module-author mono"> · {f.author}</span>{/if}
            </label>
          {/each}
          {#if extensions.length === 0}
            <p class="hint">No modules returned for {talosVersion}.</p>
          {/if}
        </div>
        {#if selectedModules.size > 0}
          <p class="hint">
            Modules:
            {#each [...selectedModules].sort() as m (m)}
              <span class="module-chip mono">{shortName(m)}</span>
            {/each}
          </p>
        {/if}
      {/if}
    </div>

    <div class="form-actions">
      <Button variant="primary" size="sm" title="Analyze this cluster's nodes and prepare a conversion plan" onclick={runPreview} disabled={busy}>
        {busy ? 'Analyzing…' : 'Analyze cluster'}
      </Button>
      <Button variant="ghost" size="sm" title="Return to the cluster detail page" onclick={() => goto(`/clusters/${cid}`)}>
        Back to cluster
      </Button>
    </div>
  {/if}

  <!-- ── Step 2: preview ───────────────────────────────────────────── -->
  {#if step === 'preview' && preview}
    <div class="header-row">
      <div>
        <h1>Conversion preview</h1>
        <p class="subtitle">
          {preview.clusterName} · Talos {preview.talosVersion} ·
          {preview.nodes.length} node(s) ({cpCount} control-plane, {workerCount} worker)
        </p>
      </div>
    </div>

    {#if !preview.canConvert || preview.blockers.length > 0}
      <div class="blockers-panel">
        <strong>This cluster cannot be converted yet.</strong>
        <ul>
          {#each preview.blockers as b (b)}
            <li>{b}</li>
          {/each}
        </ul>
      </div>
    {/if}

    <table class="data-table preview-table">
      <thead>
        <tr>
          <th>Node</th>
          <th>Role</th>
          <th>OS</th>
          <th>SSH</th>
          <th>Drivers</th>
          <th>Recommended modules</th>
          <th>Network</th>
        </tr>
      </thead>
      <tbody>
        {#each preview.nodes as n (n.name)}
          <tr>
            <td class="mono">{n.name}</td>
            <td><span class="status-badge role">{n.role}</span></td>
            <td>{n.osType === 'talos' ? 'Talos Linux' : n.osImage || '—'}</td>
            <td>
              {#if n.sshOk}
                <span class="ssh ok" title="SSH reachable">ok</span>
              {:else}
                <span class="ssh bad" title={n.sshError || 'unreachable'}>unreachable</span>
              {/if}
            </td>
            <td class="mono">{n.drivers.length ? n.drivers.join(', ') : '—'}</td>
            <td class="mono">{n.recommendedModules.length ? n.recommendedModules.map(shortName).join(', ') : '—'}</td>
            <td class="mono net">{summarizeNetwork(n)}</td>
          </tr>
        {/each}
      </tbody>
    </table>

    <div class="form-actions">
      <Button variant="secondary" size="sm" title="Add every node's recommended Image Factory modules to the selection" onclick={useRecommended} disabled={busy}>
        Use recommended modules
      </Button>
      <Button variant="ghost" size="sm" title="Return to setup" onclick={() => (step = 'setup')} disabled={busy}>
        Back
      </Button>
      <Button variant="primary" size="sm" title="Review and start the conversion" onclick={() => (step = 'confirm')} disabled={!canContinue}>
        Continue
      </Button>
    </div>
  {/if}

  <!-- ── Step 3: confirm ───────────────────────────────────────────── -->
  {#if step === 'confirm' && preview}
    <div class="header-row">
      <div>
        <h1>Confirm conversion</h1>
        <p class="subtitle">{preview.clusterName}</p>
      </div>
    </div>

    <div class="confirm-card">
      <p class="confirm-lead">
        About to convert <strong>{preview.nodes.length} node(s)</strong> to Talos
        <strong class="mono">{talosVersion}</strong>. Order: control-plane first (quorum-safe),
        then workers, one at a time.
      </p>

      {#if selectedModules.size > 0}
        <div class="chip-list">
          {#each [...selectedModules].sort() as m (m)}
            <span class="module-chip mono" title={m}>{shortName(m)}</span>
          {/each}
        </div>
      {:else}
        <p class="hint">No Image Factory modules selected.</p>
      {/if}

      <div class="warning-box">
        <strong>Warning</strong> — An etcd snapshot is taken first. During control-plane
        conversion the API server is briefly unavailable. Workers are drained before
        conversion. This cannot be undone per-node.
      </div>
    </div>

    <div class="form-actions">
      <Button variant="ghost" size="sm" title="Return to the preview" onclick={() => (step = 'preview')} disabled={busy}>
        Back
      </Button>
      <Button variant="danger" size="sm" title="Start the in-place conversion" onclick={startConversion} disabled={busy}>
        {busy ? 'Starting…' : 'Start conversion'}
      </Button>
    </div>
  {/if}

  <!-- ── Step 4: monitor ───────────────────────────────────────────── -->
  {#if step === 'monitor'}
    <div class="header-row">
      <div>
        <h1>Conversion in progress</h1>
        <p class="subtitle">{clusterName || cid}</p>
      </div>
      <div class="actions">
        {#if status?.status === 'running'}
          <Button variant="danger" size="sm" title="Cancel the running conversion job" onclick={cancelConversion} disabled={busy}>
            {busy ? 'Cancelling…' : 'Cancel'}
          </Button>
        {:else if status?.status === 'complete'}
          <Button variant="primary" size="sm" title="Mark the conversion complete" onclick={toDone}>
            Finish
          </Button>
        {/if}
      </div>
    </div>

    {#if statusError && !status}
      <div class="error-banner">{statusError}</div>
    {/if}

    {#if status}
      <div class="status-strip">
        <span class="status-badge overall {status.status}">{status.status}</span>
        <span class="phase-label">
          Phase: <strong>{status.phase}</strong>
        </span>
        <span class="phase-label" title="etcd snapshot taken before conversion; restored after">
          etcd snapshot:
          {#if status.etcdSnapshot?.taken}
            <span class="ok">taken · {formatBytes(status.etcdSnapshot.sizeBytes)}</span>
          {:else}
            <span class="pending">not yet</span>
          {/if}
        </span>
      </div>

      {#if status.status === 'failed'}
        <div class="blockers-panel">
          <strong>Conversion failed.</strong>
          <p class="hint">
            {failedNodeCount} node(s) failed. Fix the failing node and re-run from
            "Analyze cluster" on the setup step.
          </p>
          <div class="form-actions" style="margin:0.5rem 0 0">
            <Button variant="secondary" size="sm" title="Stop the job to halt retries" onclick={cancelConversion} disabled={busy}>
              Cancel job
            </Button>
            <Button variant="ghost" size="sm" title="Return to setup to re-analyze" onclick={() => (step = 'setup')} disabled={busy}>
              Back to setup
            </Button>
          </div>
        </div>
      {/if}

      <table class="data-table monitor-table">
        <thead>
          <tr>
            <th>Node</th>
            <th>Role</th>
            <th>Status</th>
            <th>Current step</th>
            <th>Error</th>
          </tr>
        </thead>
        <tbody>
          {#each status.nodes as n (n.name)}
            <tr class:failed={n.status === 'failed'}>
              <td class="mono">{n.name}</td>
              <td><span class="status-badge role">{n.role}</span></td>
              <td><span class="status-badge node {n.status}">{n.status}</span></td>
              <td class="mono">{n.currentStep || '—'}</td>
              <td class="mono err-cell">{n.error || '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>

      {#if status.stepsLog?.length}
        <details class="steps-log">
          <summary>Steps log ({status.stepsLog.length})</summary>
          <pre class="log-box mono">{status.stepsLog.join('\n')}</pre>
        </details>
      {/if}
    {:else if !statusError}
      <p class="hint">Loading conversion status…</p>
    {/if}
  {/if}

  <!-- ── Step 5: done ──────────────────────────────────────────────── -->
  {#if step === 'done'}
    <div class="header-row">
      <div>
        <h1>Conversion complete</h1>
      </div>
    </div>
    <div class="success-panel">
      <p>
        Conversion complete.
        <strong>{doneNodeCount > 0 ? doneNodeCount : (preview?.nodes.length ?? 0)}</strong>
        node(s) are now running Talos.
      </p>
      <div class="form-actions">
        <Button variant="primary" size="sm" title="Return to the cluster detail page" onclick={() => goto(`/clusters/${cid}`)}>
          View cluster
        </Button>
      </div>
    </div>
  {/if}
</div>

<style>
  .convert-wizard {
    max-width: 1100px;
    margin: 0 auto;
    animation: fade 0.15s ease;
  }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }

  .header-row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.5rem;
    flex-wrap: wrap;
  }
  .header-row h1 { margin: 0 0 0.25rem; }
  .subtitle { margin: 0; color: var(--tcs-text-muted); font-size: 0.9rem; line-height: 1.4; }
  .actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }

  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; margin: 0 0 0.75rem; }
  .hint.error { color: var(--tcs-error, #ef4444); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }

  .error-banner {
    position: relative;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem 2.25rem 0.75rem 1rem;
    color: var(--tcs-error, #ef4444);
    font-size: 0.875rem;
    margin-bottom: 1.5rem;
    word-break: break-word;
  }
  .banner-dismiss {
    position: absolute;
    top: 0.5rem;
    right: 0.6rem;
    background: none;
    border: none;
    color: var(--tcs-error, #ef4444);
    cursor: pointer;
    font-size: 1.1rem;
    line-height: 1;
  }

  .panel {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    margin-bottom: 1.25rem;
  }
  .setup-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.75rem;
    margin-bottom: 0.75rem;
  }
  .setup-grid label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; color: var(--tcs-text-muted); }
  .setup-grid select {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    background: var(--tcs-background);
    color: var(--tcs-text);
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    min-width: 0;
  }

  .module-picker {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 0.25rem 0.75rem;
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.65rem;
    margin: 0.4rem 0;
  }
  .module-option {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    font-size: 0.82rem;
    padding: 0.1rem 0;
    min-width: 0;
  }
  .module-option input { flex: 0 0 auto; }
  .module-option .module-name {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .module-option .module-author {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--tcs-text-muted);
  }
  .module-chip {
    display: inline-block;
    background: color-mix(in srgb, var(--tcs-primary) 15%, transparent);
    border: 1px solid var(--tcs-primary);
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.75rem;
    margin: 0.1rem 0.2rem 0.1rem 0;
  }

  .form-actions { display: flex; gap: 0.75rem; margin-top: 1rem; flex-wrap: wrap; }

  .data-table { width: 100%; border-collapse: collapse; margin-bottom: 0.75rem; }
  .data-table th, .data-table td { text-align: left; padding: 0.6rem 0.75rem; border-bottom: 1px solid var(--tcs-border); font-size: 0.85rem; vertical-align: top; }
  .data-table th { color: var(--tcs-text-muted); font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.05em; }
  .data-table tr:hover { background: var(--tcs-surface-hover); }
  .preview-table .net { max-width: 24rem; }

  .status-badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    text-transform: capitalize;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    white-space: nowrap;
  }
  .status-badge.role { color: var(--tcs-accent, #4a9eff); border-color: var(--tcs-accent, #4a9eff); }
  .status-badge.overall.running, .status-badge.node.cordon,
  .status-badge.node.drain, .status-badge.node.kexec, .status-badge.node.install,
  .status-badge.node.reboot, .status-badge.node.join, .status-badge.node.recover {
    color: var(--tcs-info, #8b7cf6); border-color: var(--tcs-info, #8b7cf6);
  }
  .status-badge.overall.running { color: var(--tcs-warning); border-color: var(--tcs-warning); }
  .status-badge.overall.complete, .status-badge.node.done, .status-badge.node.skipped {
    color: var(--tcs-success); border-color: var(--tcs-success);
  }
  .status-badge.overall.failed, .status-badge.node.failed {
    color: var(--tcs-error); border-color: var(--tcs-error);
  }
  .status-badge.overall.cancelled, .status-badge.node.pending {
    color: var(--tcs-text-muted); border-color: var(--tcs-border);
  }
  .monitor-table tr.failed td { background: rgba(239, 68, 68, 0.08); }
  .err-cell { color: var(--tcs-error, #ef4444); max-width: 24rem; word-break: break-word; }

  .ssh { font-size: 0.8rem; font-weight: 600; }
  .ssh.ok { color: var(--tcs-success); }
  .ssh.bad { color: var(--tcs-error); }

  .blockers-panel {
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.35);
    border-radius: 8px;
    padding: 0.85rem 1rem;
    margin-bottom: 1.25rem;
    color: var(--tcs-text);
    font-size: 0.875rem;
  }
  .blockers-panel ul { margin: 0.5rem 0 0; padding-left: 1.25rem; }
  .blockers-panel li { margin: 0.15rem 0; color: var(--tcs-error, #ef4444); word-break: break-word; }

  .confirm-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
    margin-bottom: 1.25rem;
  }
  .confirm-lead { margin: 0 0 1rem; font-size: 0.95rem; line-height: 1.5; }
  .chip-list { margin-bottom: 1rem; }
  .warning-box {
    background: rgba(245, 158, 11, 0.08);
    border: 1px solid rgba(245, 158, 11, 0.4);
    border-radius: 6px;
    padding: 0.85rem 1rem;
    font-size: 0.85rem;
    line-height: 1.5;
    color: var(--tcs-warning, #f59e0b);
  }

  .status-strip {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
    padding: 0.75rem 1rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
  }
  .phase-label { color: var(--tcs-text-muted); font-size: 0.875rem; }
  .phase-label strong { color: var(--tcs-text); }
  .phase-label .ok { color: var(--tcs-success); }
  .phase-label .pending { color: var(--tcs-warning); }

  .steps-log { margin-top: 1rem; }
  .steps-log summary { cursor: pointer; font-weight: 600; font-size: 0.9rem; }
  .log-box {
    margin: 0.75rem 0 0;
    padding: 0.75rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    font-size: 0.75rem;
    overflow: auto;
    max-height: 18rem;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .success-panel {
    background: rgba(34, 197, 94, 0.08);
    border: 1px solid rgba(34, 197, 94, 0.4);
    border-radius: 8px;
    padding: 1.25rem 1.5rem;
    margin-bottom: 1.25rem;
    color: var(--tcs-success);
  }
  .success-panel p { margin: 0 0 0.25rem; font-size: 1.05rem; font-weight: 600; }
</style>
