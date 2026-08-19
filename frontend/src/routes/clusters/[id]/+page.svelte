<script lang="ts">
  import { page } from '$app/stores';
  import { client } from '$lib/api/client';
  import { onMount } from 'svelte';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  import {
    formatBytes,
    isControlPlane,
    machineLabel,
    machineHasBmc,
    type Machine,
    type ClusterBackup,
  } from '$lib/api/types';

  type Tab = 'nodes' | 'machines' | 'config' | 'backups';

  let cluster = $state<any>(null);
  let loading = $state(true);
  let error = $state('');
  let busy = $state(false);
  let tab = $state<Tab>('nodes');

  // ── machines / nodes (same inventory source) ──────────────────────
  let machines = $state<Machine[]>([]);
  let machinesLoading = $state(false);
  let machinesError = $state('');

  // ── config patches ────────────────────────────────────────────────
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
  let configLoading = $state(false);
  let configError = $state('');
  let showEditor = $state(false);
  let newPatch = $state({ path: '', value: '', priority: 0 });
  let saving = $state(false);
  let applying = $state(false);
  let lastApply = $state<{
    dryRun?: boolean;
    count?: number;
    appliedTo?: string[];
    errors?: string[];
    documents?: Array<{ address?: string; patchPreview?: string }>;
  } | null>(null);

  // ── backups ───────────────────────────────────────────────────────
  let backups = $state<ClusterBackup[]>([]);
  let backupsLoading = $state(false);
  let backupsError = $state('');
  let creating = $state(false);
  let restoringId = $state<string | null>(null);
  let restoreMachineId = $state('');
  let scheduleHours = $state(0);
  let retention = $state(10);
  let savingSchedule = $state(false);

  // ── header actions ────────────────────────────────────────────────
  let talosconfigText = $state('');
  let kubeconfigText = $state('');
  let talosTestResults = $state<
    Array<{ address?: string; ok: boolean; talosVersion?: string; error?: string }>
  >([]);
  let upgradeImage = $state('ghcr.io/siderolabs/installer:v1.9.0');
  let upgradeMaxUnavail = $state(1);
  let upgradeCpLast = $state(true);
  let desiredWorkers = $state(0);

  const cid = $page.params.id;

  async function loadCluster() {
    cluster = await client.get(`/clusters/${cid}`);
    desiredWorkers = cluster.workerSize ?? cluster.worker_size ?? 0;
    scheduleHours = cluster.backupScheduleHours ?? 0;
    retention = cluster.backupRetention ?? 10;
  }

  async function loadMachines() {
    machinesLoading = true;
    machinesError = '';
    try {
      const data = (await client.get(`/clusters/${cid}/machines`)) as Machine[];
      machines = Array.isArray(data) ? data : [];
    } catch (e: unknown) {
      machinesError = e instanceof Error ? e.message : 'Failed to load machines';
    } finally {
      machinesLoading = false;
    }
  }

  async function loadConfig() {
    configLoading = true;
    configError = '';
    try {
      const data = (await client.get(`/clusters/${cid}/config`)) as ConfigPatch[];
      patches = Array.isArray(data) ? data : [];
    } catch (e: unknown) {
      configError = e instanceof Error ? e.message : 'Failed to load config patches';
    } finally {
      configLoading = false;
    }
  }

  async function loadBackups() {
    backupsLoading = true;
    backupsError = '';
    try {
      const data = (await client.get(`/clusters/${cid}/backups`)) as ClusterBackup[];
      backups = Array.isArray(data) ? data : [];
      const cps = machines.filter(isControlPlane);
      if (cps.length && !restoreMachineId) restoreMachineId = cps[0].id;
    } catch (e: unknown) {
      backupsError = e instanceof Error ? e.message : 'Failed to load backups';
    } finally {
      backupsLoading = false;
    }
  }

  function selectTab(t: Tab) {
    tab = t;
    if (t === 'machines' && machines.length === 0 && !machinesLoading) void loadMachines();
    if (t === 'config' && patches.length === 0 && !configLoading) void loadConfig();
    if (t === 'backups' && backups.length === 0 && !backupsLoading) void loadBackups();
  }

  onMount(async () => {
    try {
      await loadCluster();
      await loadMachines();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load cluster';
    } finally {
      loading = false;
    }
  });

  // ── header action handlers ────────────────────────────────────────
  async function refresh() {
    busy = true;
    try {
      const res = (await client.post(`/clusters/${cid}/refresh`, {})) as { machines: number };
      await loadCluster();
      await loadMachines();
      success(`Refreshed ${res.machines} machine(s) from kubeconfig`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Refresh failed (need stored kubeconfig)');
    } finally {
      busy = false;
    }
  }

  async function testTalos() {
    busy = true;
    talosTestResults = [];
    try {
      const res = (await client.post(`/clusters/${cid}/talos/test`, {})) as {
        results: Array<{ ok: boolean; address?: string; talosVersion?: string; error?: string }>;
      };
      talosTestResults = res.results ?? [];
      const ok = talosTestResults.filter((r) => r.ok).length;
      const fail = talosTestResults.length - ok;
      if (fail === 0) success(`Talos test: ${ok} ok`);
      else notifyError(`Talos test: ${ok} ok, ${fail} failed`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Talos connectivity test failed');
    } finally {
      busy = false;
    }
  }

  async function saveKubeconfig() {
    if (!kubeconfigText.trim()) return;
    busy = true;
    try {
      await client.put(`/clusters/${cid}/kubeconfig`, { kubeconfig: kubeconfigText });
      await loadCluster();
      kubeconfigText = '';
      success('Kubeconfig saved (encrypted at rest)');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save kubeconfig');
    } finally {
      busy = false;
    }
  }

  async function probeVersions() {
    busy = true;
    try {
      const res = (await client.post(`/clusters/${cid}/talos/versions`, {})) as {
        ok: number;
        failed: number;
        versions?: string[];
      };
      await loadCluster();
      success(
        `Version probe: ${res.ok} ok, ${res.failed} failed` +
          (res.versions?.length ? ` (${res.versions.join(', ')})` : '')
      );
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Version probe failed');
    } finally {
      busy = false;
    }
  }

  async function saveTalosconfig() {
    if (!talosconfigText.trim()) return;
    busy = true;
    try {
      await client.put(`/clusters/${cid}/talosconfig`, { talosconfig: talosconfigText });
      await loadCluster();
      talosconfigText = '';
      success('Talosconfig saved (encrypted at rest)');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save talosconfig');
    } finally {
      busy = false;
    }
  }

  async function startClusterUpgrade() {
    if (!upgradeImage.trim()) {
      notifyError('Installer image is required');
      return;
    }
    if (!confirm(`Start rolling upgrade of this cluster to ${upgradeImage}?`)) return;
    busy = true;
    try {
      const res = (await client.post(`/clusters/${cid}/upgrade`, {
        image: upgradeImage.trim(),
        maxUnavailable: upgradeMaxUnavail,
        controlPlaneLast: upgradeCpLast,
      })) as { job?: { id: string } };
      success(`Upgrade job queued${res.job?.id ? `: ${res.job.id}` : ''}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to start cluster upgrade');
    } finally {
      busy = false;
    }
  }

  async function scaleWorkers() {
    if (!confirm(`Set desired worker size to ${desiredWorkers}? (inventory only)`)) return;
    busy = true;
    try {
      await client.post(`/clusters/${cid}/scale`, { desiredWorkers: Number(desiredWorkers) });
      await loadCluster();
      success(`Desired workers set to ${desiredWorkers}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Scale failed');
    } finally {
      busy = false;
    }
  }

  // ── config patch handlers ─────────────────────────────────────────
  async function addPatch() {
    if (!newPatch.path.trim()) return;
    saving = true;
    try {
      await client.post(`/clusters/${cid}/config`, newPatch);
      showEditor = false;
      newPatch = { path: '', value: '', priority: 0 };
      success('Config patch added');
      await loadConfig();
    } catch {
      notifyError('Failed to add patch');
    } finally {
      saving = false;
    }
  }

  async function deletePatch(id: string) {
    try {
      await client.delete(`/clusters/${cid}/config/${id}`);
      patches = patches.filter((p) => p.id !== id);
      success('Patch removed');
    } catch {
      notifyError('Failed to remove patch');
    }
  }

  async function applyAll(dryRun = false) {
    applying = true;
    lastApply = null;
    try {
      const res = (await client.post(`/clusters/${cid}/config/apply`, { dryRun })) as {
        count: number;
        appliedTo?: string[];
        dryRun?: boolean;
        errors?: string[];
        documents?: Array<{ address?: string; patchPreview?: string }>;
      };
      lastApply = res;
      const errN = res.errors?.length ?? 0;
      if (errN > 0) {
        notifyError(dryRun ? `Dry-run: ${res.count} ok, ${errN} error(s)` : `Applied ${res.count}; ${errN} error(s)`);
      } else {
        success(dryRun ? `Dry-run OK for ${res.count} machine(s)` : `Applied patches to ${res.count} machine(s)`);
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to apply patches via Talos API');
    } finally {
      applying = false;
    }
  }

  // ── backup handlers ───────────────────────────────────────────────
  async function createBackup() {
    creating = true;
    try {
      const backup = (await client.post(`/clusters/${cid}/backups`, {
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
      const res = await fetch(`/api/clusters/${cid}/backups/${backup.id}/download`, {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
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
      await client.delete(`/clusters/${cid}/backups/${backup.id}`);
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
      const res = (await client.post(`/clusters/${cid}/backups/${backup.id}/restore`, {
        confirm: true,
        runBootstrap: true,
        skipHashCheck: false,
        machineId: restoreMachineId || null,
      })) as { ok?: boolean; message?: string; bootstrapError?: string };
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
      await client.put(`/clusters/${cid}/backups/schedule`, {
        scheduleHours: scheduleHours > 0 ? scheduleHours : null,
        retention: retention > 0 ? retention : 10,
      });
      success(
        scheduleHours > 0
          ? `Auto-backup every ${scheduleHours}h (keep ${retention})`
          : 'Auto-backup disabled'
      );
      await loadCluster();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save schedule');
    } finally {
      savingSchedule = false;
    }
  }

  const controlPlanes = $derived(machines.filter(isControlPlane));
  const cpCount = $derived(machines.filter(isControlPlane).length);
  const workerCount = $derived(machines.length - cpCount);
</script>

<div class="cluster-detail">
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error-banner">{error}</div>
  {:else if cluster}
    <div class="header-row">
      <h1>{cluster.name}</h1>
      <div class="actions">
        <Button variant="secondary" size="sm" onclick={refresh} disabled={busy}>Refresh from K8s</Button>
        <Button variant="secondary" size="sm" onclick={testTalos} disabled={busy}>Test Talos</Button>
        <Button variant="secondary" size="sm" onclick={probeVersions} disabled={busy}>Probe versions</Button>
      </div>
    </div>

    <div class="info-grid">
      <div class="info-item">
        <span class="info-label">Kubernetes</span>
        <span class="info-value">{cluster.controlPlaneVersion || cluster.control_plane_version || '—'}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Talos</span>
        <span class="info-value">{cluster.talosVersion || cluster.talos_version || '—'}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Status</span>
        <span class="info-value"><span class="status-badge {cluster.status}">{cluster.status}</span></span>
      </div>
      <div class="info-item">
        <span class="info-label">Nodes</span>
        <span class="info-value">{cpCount} control-plane · {workerCount} worker</span>
      </div>
      <div class="info-item">
        <span class="info-label">Talosconfig</span>
        <span class="info-value">{cluster.hasTalosconfig || cluster.has_talosconfig ? 'Attached' : 'Missing'}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Kubeconfig</span>
        <span class="info-value">{cluster.hasKubeconfig || cluster.has_kubeconfig ? 'Stored' : 'Missing'}</span>
      </div>
    </div>

    {#if talosTestResults.length > 0}
      <div class="talos-test">
        <div class="talos-test-header">
          <h3>Talos connectivity</h3>
          <span class="hint">
            {talosTestResults.filter((r) => r.ok).length} ok ·
            {talosTestResults.filter((r) => !r.ok).length} failed
          </span>
          <Button variant="ghost" size="sm" class="dismiss" onclick={() => (talosTestResults = [])}>Dismiss</Button>
        </div>
        <table class="data-table">
          <thead>
            <tr>
              <th>Address</th>
              <th>Result</th>
              <th>Talos version</th>
            </tr>
          </thead>
          <tbody>
            {#each talosTestResults as r (r.address ?? Math.random())}
              <tr>
                <td class="mono">{r.address || '—'}</td>
                <td>
                  {#if r.ok}
                    <span class="status-badge running">ok</span>
                  {:else}
                    <span class="status-badge offline" title={r.error}>failed</span>
                  {/if}
                </td>
                <td>{r.talosVersion || (r.error ? '' : '—')}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if talosTestResults.some((r) => !r.ok)}
          <details>
            <summary>Failure details</summary>
            <ul class="err-list">
              {#each talosTestResults.filter((r) => !r.ok) as r (r.address ?? Math.random())}
                <li><span class="mono">{r.address}</span>: {r.error}</li>
              {/each}
            </ul>
          </details>
        {/if}
      </div>
    {/if}

    <details class="panel">
      <summary>Cluster actions</summary>
      <div class="actions-stack">
        <div class="action-block">
          <h3>Attach / replace talosconfig</h3>
          <textarea bind:value={talosconfigText} rows="6" placeholder="Paste ~/.talos/config YAML"></textarea>
          <Button variant="primary" size="sm" onclick={saveTalosconfig} disabled={busy || !talosconfigText.trim()}>
            Save talosconfig
          </Button>
        </div>
        <div class="action-block">
          <h3>Attach / replace kubeconfig</h3>
          <p class="hint">Needed for "Refresh from K8s" (live node discovery + version sync).</p>
          <textarea bind:value={kubeconfigText} rows="6" placeholder="Paste ~/.kube/config YAML"></textarea>
          <Button variant="primary" size="sm" onclick={saveKubeconfig} disabled={busy || !kubeconfigText.trim()}>
            Save kubeconfig
          </Button>
        </div>
        <div class="action-block">
          <h3>Scale workers (inventory target)</h3>
          <div class="inline-form">
            <label>
              Desired workers
              <input type="number" min="0" max="500" bind:value={desiredWorkers} />
            </label>
            <Button variant="primary" size="sm" onclick={scaleWorkers} disabled={busy}>
              Update desired size
            </Button>
            <span class="hint">Does not provision metal — update inventory only</span>
          </div>
        </div>
        <div class="action-block">
          <h3>Rolling Talos upgrade</h3>
          <div class="inline-form">
            <label>
              Installer image
              <input type="text" bind:value={upgradeImage} placeholder="ghcr.io/siderolabs/installer:v1.x" />
            </label>
            <label class="num">
              Max unavailable
              <input type="number" min="1" max="20" bind:value={upgradeMaxUnavail} />
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={upgradeCpLast} />
              Workers first (control plane last)
            </label>
            <Button variant="primary" size="sm" onclick={startClusterUpgrade} disabled={busy}>
              Start rolling upgrade
            </Button>
            <a class="jobs-link" href="/upgrades">View upgrade jobs →</a>
          </div>
        </div>
      </div>
    </details>

    <nav class="tabs">
      <button class:active={tab === 'nodes'} onclick={() => selectTab('nodes')}>Nodes</button>
      <button class:active={tab === 'machines'} onclick={() => selectTab('machines')}>Machines</button>
      <button class:active={tab === 'config'} onclick={() => selectTab('config')}>Config</button>
      <button class:active={tab === 'backups'} onclick={() => selectTab('backups')}>Backups</button>
    </nav>

    <!-- ── Nodes ─────────────────────────────────────────────────── -->
    {#if tab === 'nodes'}
      <div class="tab-panel">
        {#if machinesLoading}
          <Spinner />
        {:else if machinesError}
          <div class="error-banner">{machinesError}</div>
        {:else if machines.length === 0}
          <div class="empty-state"><p>No nodes in this cluster</p></div>
        {:else}
          <table class="data-table">
            <thead>
              <tr>
                <th>Node</th>
                <th>Role</th>
                <th>Status</th>
                <th>Address</th>
                <th>Talos</th>
              </tr>
            </thead>
            <tbody>
              {#each machines as m (m.id)}
                <tr>
                  <td class="mono">{m.hostname || machineLabel(m)}</td>
                  <td>{m.machineType || '—'}</td>
                  <td><span class="status-badge {m.status}">{m.status}</span></td>
                  <td class="mono">{m.address || '—'}</td>
                  <td>{m.talosVersion || '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    {/if}

    <!-- ── Machines ──────────────────────────────────────────────── -->
    {#if tab === 'machines'}
      <div class="tab-panel">
        <p class="hint">
          Inventory for this cluster. Click a machine to edit role, MAC, BMC, address, or install disk.
          <a href="/machines/import">Import CSV/YAML</a>
        </p>
        {#if machinesLoading}
          <Spinner />
        {:else if machinesError}
          <div class="error-banner">{machinesError}</div>
        {:else if machines.length === 0}
          <div class="empty-state">
            <p>No machines assigned to this cluster</p>
            <p class="hint">Import inventory or register machines from the provision wizard.</p>
          </div>
        {:else}
          <table class="data-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Status</th>
                <th>Role</th>
                <th>MAC</th>
                <th>Address</th>
                <th>BMC</th>
                <th>Talos</th>
              </tr>
            </thead>
            <tbody>
              {#each machines as m (m.id)}
                <tr>
                  <td><a class="mono" href="/machines/{m.id}">{m.hostname || machineLabel(m)}</a></td>
                  <td><span class="status-badge {m.status}">{m.status}</span></td>
                  <td>{m.machineType || '—'}</td>
                  <td class="mono">{m.macAddress || '—'}</td>
                  <td class="mono">{m.address || '—'}</td>
                  <td>{machineHasBmc(m) ? 'yes' : '—'}</td>
                  <td>{m.talosVersion || '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    {/if}

    <!-- ── Config ────────────────────────────────────────────────── -->
    {#if tab === 'config'}
      <div class="tab-panel">
        <div class="panel-header">
          <p class="hint">
            Patches are stored in TCS, merged into each node's live machine config (COSI Get), then applied
            with Talos <code>ApplyConfiguration</code> (no-reboot, pure-Rust). Requires a talosconfig and
            reachability to node :50000.
          </p>
          <div class="actions">
            <Button variant="ghost" size="sm" onclick={() => applyAll(true)} disabled={applying || patches.length === 0}>Dry-run</Button>
            <Button variant="secondary" size="sm" onclick={() => applyAll(false)} disabled={applying || patches.length === 0}>
              {applying ? 'Applying…' : 'Apply to cluster'}
            </Button>
            <Button variant="primary" size="sm" onclick={() => (showEditor = !showEditor)}>
              {showEditor ? 'Cancel' : 'Add Patch'}
            </Button>
          </div>
        </div>

        {#if lastApply}
          <div class="apply-result">
            <h3>{lastApply.dryRun ? 'Dry-run result' : 'Apply result'}</h3>
            <p>
              {lastApply.count ?? 0} machine(s)
              {#if lastApply.errors?.length} · {lastApply.errors.length} error(s){/if}
            </p>
            {#if lastApply.documents?.[0]?.patchPreview}
              <details><summary>Patch preview</summary><pre>{lastApply.documents[0].patchPreview}</pre></details>
            {/if}
            {#if lastApply.errors?.length}
              <details open><summary>Errors</summary><ul>{#each lastApply.errors as err}<li class="err-line">{err}</li>{/each}</ul></details>
            {/if}
            {#if lastApply.appliedTo?.length}
              <details><summary>Applied to</summary><ul>{#each lastApply.appliedTo as line}<li>{line}</li>{/each}</ul></details>
            {/if}
          </div>
        {/if}

        {#if showEditor}
          <div class="patch-editor">
            <div class="form-row">
              <div class="form-group">
                <label>Document Path</label>
                <input type="text" bind:value={newPatch.path} placeholder="/machine/sysctls/net.ipv4.ip_forward" />
              </div>
              <div class="form-group narrow">
                <label>Priority</label>
                <input type="number" bind:value={newPatch.priority} />
              </div>
            </div>
            <div class="form-group">
              <label>Value (YAML)</label>
              <textarea bind:value={newPatch.value} rows="5" placeholder="true"></textarea>
            </div>
            <div class="editor-actions">
              <Button variant="primary" size="sm" onclick={addPatch} disabled={saving}>
                {saving ? 'Saving…' : 'Apply Patch'}
              </Button>
            </div>
          </div>
        {/if}

        {#if configLoading}
          <Spinner />
        {:else if configError}
          <div class="error-banner">{configError}</div>
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
    {/if}

    <!-- ── Backups ───────────────────────────────────────────────── -->
    {#if tab === 'backups'}
      <div class="tab-panel">
        <div class="panel-header">
          <h2>Etcd snapshots</h2>
          <Button variant="primary" size="sm" onclick={createBackup} disabled={creating}>
            {creating ? 'Creating…' : 'Create Backup'}
          </Button>
        </div>

        <section class="panel">
          <h3>Schedule</h3>
          <p class="hint">Automatic etcd snapshots when the cluster has a talosconfig. Scheduler checks about every 15 minutes.</p>
          <div class="inline-form">
            <label>
              Interval (hours, 0 = off)
              <input type="number" min="0" max="168" bind:value={scheduleHours} />
            </label>
            <label>
              Retention (ready backups to keep)
              <input type="number" min="1" max="100" bind:value={retention} />
            </label>
            <Button variant="secondary" size="sm" onclick={saveSchedule} disabled={savingSchedule}>
              {savingSchedule ? 'Saving…' : 'Save schedule'}
            </Button>
          </div>
          <p class="hint">
            Last automatic backup:
            {#if cluster.lastAutoBackupAt}
              <strong>{new Date(cluster.lastAutoBackupAt).toLocaleString()}</strong>
            {:else}
              <em>never</em>
            {/if}
          </p>
        </section>

        <section class="panel">
          <h3>Restore target</h3>
          <p class="hint">Control-plane node used for disaster recovery restore.</p>
          <select bind:value={restoreMachineId}>
            <option value="">Auto (first control-plane / talosconfig endpoint)</option>
            {#each controlPlanes as m (m.id)}
              <option value={m.id}>{machineLabel(m)} — {m.address || 'no address'}</option>
            {/each}
          </select>
        </section>

        {#if backupsLoading}
          <Spinner />
        {:else if backupsError}
          <div class="error-banner">{backupsError}</div>
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
                    <div class="row-actions">
                      <Button variant="ghost" size="sm" onclick={() => downloadBackup(backup)} disabled={backup.status !== 'ready'}>Download</Button>
                      <Button variant="secondary" size="sm" onclick={() => restoreBackup(backup)} disabled={backup.status !== 'ready' || restoringId === backup.id}>
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
    {/if}
  {/if}
</div>

<style>
  .cluster-detail h1 { margin: 0; }
  .header-row { display: flex; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1.5rem; flex-wrap: wrap; }
  .actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .info-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }
  .info-item { display: flex; flex-direction: column; gap: 0.25rem; }
  .info-label { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .info-value { font-size: 1.05rem; font-weight: 500; }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem 1rem;
    color: var(--tcs-error, #ef4444);
    font-size: 0.875rem;
    margin-bottom: 1.5rem;
  }

  .talos-test {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin-bottom: 1.5rem;
  }
  .talos-test-header { display: flex; align-items: center; gap: 1rem; margin-bottom: 0.75rem; }
  .talos-test-header h3 { margin: 0; font-size: 1rem; }
  .talos-test-header .hint { margin: 0; }
  .talos-test-header .dismiss { margin-left: auto; }
  .err-list { margin: 0.5rem 0 0; padding-left: 1.25rem; font-size: 0.8rem; color: var(--tcs-text-muted); }
  .err-list li { margin-bottom: 0.25rem; word-break: break-word; }
  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; margin: 0 0 0.75rem; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }

  .panel {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    margin-bottom: 1.25rem;
  }
  .panel summary { cursor: pointer; font-weight: 600; }
  .panel h2 { margin: 0 0 0.5rem; font-size: 1rem; }
  .panel h3 { margin: 0 0 0.5rem; font-size: 0.9rem; }
  .actions-stack { display: flex; flex-direction: column; gap: 1.25rem; margin-top: 0.75rem; }
  .action-block { display: flex; flex-direction: column; gap: 0.5rem; }
  .action-block textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem;
    color: var(--tcs-text);
  }
  .inline-form { display: flex; flex-wrap: wrap; gap: 0.75rem 1rem; align-items: flex-end; }
  .inline-form label { display: flex; flex-direction: column; gap: 0.3rem; font-size: 0.8rem; color: var(--tcs-text-muted); }
  .inline-form label.num { min-width: 110px; }
  .inline-form label.check { flex-direction: row; align-items: center; color: var(--tcs-text); }
  .inline-form input[type='text'],
  .inline-form input[type='number'] {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
    color: var(--tcs-text);
    min-width: 10rem;
  }
  .jobs-link { font-size: 0.85rem; color: var(--tcs-secondary); align-self: center; }

  .tabs { display: flex; gap: 0; border-bottom: 1px solid var(--tcs-border); margin-bottom: 1.5rem; }
  .tabs button {
    padding: 0.75rem 1.25rem;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--tcs-text-muted);
    font-size: 0.95rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }
  .tabs button:hover { color: var(--tcs-text); }
  .tabs button.active { color: var(--tcs-text); border-bottom-color: var(--tcs-primary); }

  .tab-panel { animation: fade 0.15s ease; }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  .panel-header { display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; margin-bottom: 1rem; flex-wrap: wrap; }
  .panel-header h2 { margin: 0 0 0.25rem; font-size: 1rem; }
  .empty-state { text-align: center; padding: 3rem; color: var(--tcs-text-muted); }

  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td { text-align: left; padding: 0.7rem 0.75rem; border-bottom: 1px solid var(--tcs-border); font-size: 0.875rem; }
  .data-table th { color: var(--tcs-text-muted); font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; }
  .data-table tr:hover { background: var(--tcs-surface-hover); }
  .row-actions { display: flex; gap: 0.35rem; flex-wrap: wrap; }

  .status-badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    text-transform: capitalize;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
  }
  .status-badge.running, .status-badge.ready { color: var(--tcs-success); border-color: var(--tcs-success); }
  .status-badge.offline, .status-badge.failed { color: var(--tcs-error); border-color: var(--tcs-error); }
  .status-badge.pending, .status-badge.unknown { color: var(--tcs-warning); border-color: var(--tcs-warning); }

  .apply-result {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin-bottom: 1.5rem;
  }
  .apply-result h3 { margin: 0 0 0.5rem; font-size: 1rem; }
  .apply-result pre { background: var(--tcs-background); padding: 0.75rem; border-radius: 6px; overflow: auto; font-size: 0.8rem; }
  .apply-result ul { margin: 0.5rem 0 0; padding-left: 1.25rem; }
  .apply-result .err-line { color: var(--tcs-error, #f87171); font-size: 0.85rem; word-break: break-word; }

  .patch-editor {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
    margin-bottom: 1.5rem;
  }
  .form-row { display: flex; gap: 1rem; margin-bottom: 1rem; }
  .form-group { display: flex; flex-direction: column; gap: 0.4rem; flex: 1; }
  .form-group.narrow { flex: 0 0 100px; }
  .form-group label { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .form-group input, .form-group textarea {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    font-family: ui-monospace, monospace;
    font-size: 0.875rem;
  }
  .form-group textarea { resize: vertical; }
  .editor-actions { display: flex; justify-content: flex-end; margin-top: 1rem; }

  .patches-list { display: flex; flex-direction: column; gap: 0.75rem; }
  .patch-card { background: var(--tcs-surface); border: 1px solid var(--tcs-border); border-radius: 8px; overflow: hidden; }
  .patch-header { display: flex; align-items: center; gap: 0.75rem; padding: 0.75rem 1rem; border-bottom: 1px solid var(--tcs-border); }
  .patch-path { font-size: 0.8rem; color: var(--tcs-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .scope-badge { font-size: 0.65rem; padding: 0.15rem 0.4rem; border-radius: 4px; text-transform: uppercase; }
  .scope-badge.cluster { background: rgba(79, 139, 255, 0.15); color: var(--tcs-secondary); }
  .scope-badge.machine { background: rgba(245, 158, 11, 0.15); color: var(--tcs-warning); }
  .priority { font-size: 0.75rem; color: var(--tcs-text-muted); margin-left: auto; }
  .patch-value { margin: 0; padding: 1rem; font-size: 0.8rem; line-height: 1.6; overflow-x: auto; color: var(--tcs-text-muted); }

  select {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
    min-width: 16rem;
  }
</style>
