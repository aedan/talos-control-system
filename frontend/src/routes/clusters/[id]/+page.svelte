<script lang="ts">
  import { page } from '$app/stores';
  import { client } from '$lib/api/client';
  import { onMount, onDestroy } from 'svelte';
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
    type FactoryExtension,
  } from '$lib/api/types';
  import WorkloadsExplorer from '$lib/explorer/WorkloadsExplorer.svelte';

  type Tab = 'machines' | 'config' | 'backups';

  let cluster = $state<any>(null);
  let loading = $state(true);
  let error = $state('');
  let busy = $state(false);
  let tab = $state<Tab>('machines');
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  // ── cluster default modules (Image Factory) ────────────────────────
  let factoryExtensions = $state<FactoryExtension[]>([]);
  let factoryBusy = $state(false);
  let factoryError = $state('');
  let clusterModules = $state<Set<string>>(new Set());
  let modulesDirty = $state(false);
  function shortModuleName(full: string): string {
    const i = full.indexOf('/');
    return i >= 0 ? full.slice(i + 1) : full;
  }
  function toggleClusterModule(name: string) {
    const next = new Set(clusterModules);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    clusterModules = next;
    const cur = cluster?.factoryModules || [];
    modulesDirty = [...next].sort().join('|') !== [...cur].sort().join('|');
  }
  async function loadClusterModules() {
    if (!cluster) return;
    clusterModules = new Set(cluster.factoryModules || []);
    modulesDirty = false;
    factoryBusy = true;
    factoryError = '';
    try {
      const res = await client.get(`/factory/extensions?version=${encodeURIComponent(cluster.talosVersion || cluster.talos_version || 'v1.13.7')}`);
      factoryExtensions = ((res as { extensions: FactoryExtension[] }).extensions) || [];
    } catch (e: unknown) {
      factoryError = e instanceof Error ? e.message : 'Failed to load module catalog';
      factoryExtensions = [];
    } finally {
      factoryBusy = false;
    }
  }
  async function saveClusterModules() {
    if (!cluster || busy) return;
    busy = true;
    try {
      await client.put(`/clusters/${$page.params.id}/modules`, { modules: [...clusterModules] });
      cluster.factoryModules = [...clusterModules];
      modulesDirty = false;
      success('Cluster modules updated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to update cluster modules');
    } finally {
      busy = false;
    }
  }

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

  // ── rolling upgrade (derived image: talos version + modules + k8s) ──
  let upgradeTalosVersion = $state('');
  let upgradeK8sVersion = $state('');
  let upgradeMaxUnavail = $state(1);
  let upgradeCpLast = $state(true);
  let talosVersions = $state<string[]>([]);
  let currentTalos = $state('');
  let k8sCurrent = $state('');
  let k8sSupported = $state<string[]>([]);
  let targetsBusy = $state(false);
  let targetsError = $state('');
  let targetsNote = $state('');

  // ── upgrade jobs (per-cluster) ────────────────────────────────────
  interface UpgradeJob {
    id: string;
    scope: string;
    image: string;
    status: string;
    createdAt?: string;
    created_at?: string;
  }
  let upgradeJobs = $state<UpgradeJob[]>([]);
  let upgradeJobsLoading = $state(false);
  let upgradeJobDetail = $state<any>(null);

  const cid = $page.params.id;

  async function loadCluster() {
    cluster = await client.get(`/clusters/${cid}`);
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

  async function loadUpgradeJobs() {
    upgradeJobsLoading = true;
    try {
      const data = (await client.get(`/clusters/${cid}/upgrade-jobs`)) as UpgradeJob[];
      upgradeJobs = Array.isArray(data) ? data : [];
    } catch {
      upgradeJobs = [];
    } finally {
      upgradeJobsLoading = false;
    }
  }

  async function openUpgradeJob(id: string) {
    try {
      upgradeJobDetail = await client.get(`/upgrade-jobs/${id}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load job');
    }
  }

  async function cancelUpgradeJob(id: string) {
    if (!confirm('Request cancel for this upgrade job?')) return;
    busy = true;
    try {
      await client.post(`/upgrade-jobs/${id}/cancel`, {});
      success('Cancel requested');
      await loadUpgradeJobs();
      if (upgradeJobDetail?.job?.id === id) upgradeJobDetail = await client.get(`/upgrade-jobs/${id}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Cancel failed');
    } finally {
      busy = false;
    }
  }

  onMount(async () => {
    try {
      await loadCluster();
      await loadMachines();
      loadUpgradeJobs();
      // Load the Image Factory module catalog once; the poll below must NOT
      // re-fetch it (doing so reset the selection + <details> state every 15s).
      void loadClusterModules();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load cluster';
    } finally {
      loading = false;
    }
    // Only the physical nodes (and their status) refresh on the poll interval.
    // The cluster object + default-modules section are loaded once and left
    // alone so the user can interact with them without the page resetting.
    pollTimer = setInterval(() => {
      loadMachines();
      loadUpgradeJobs();
    }, 15000);
  });

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
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

  async function loadUpgradeTargets() {
    targetsBusy = true;
    targetsError = '';
    targetsNote = '';
    try {
      const res = (await client.get(`/clusters/${cid}/upgrade-targets`)) as {
        talos: { current: string; versions: string[] };
        k8s: { current: string; supported: string[]; note?: string };
        notes?: string[];
      };
      currentTalos = res.talos.current || '';
      talosVersions = res.talos.versions || [];
      k8sCurrent = res.k8s.current || '';
      k8sSupported = res.k8s.supported || [];
      upgradeTalosVersion = currentTalos;
      upgradeK8sVersion = '';
      const noteBits: string[] = [];
      if (res.notes?.length) noteBits.push(...res.notes);
      if (res.k8s.note) noteBits.push(res.k8s.note);
      targetsNote = noteBits.join(' ');
    } catch (e: unknown) {
      targetsError = e instanceof Error ? e.message : 'Failed to load upgrade targets';
    } finally {
      targetsBusy = false;
    }
  }

  $effect(() => {
    if (cluster && !talosVersions.length && !targetsBusy && !targetsError) {
      loadUpgradeTargets();
    }
  });

  function upgradeSummary(): string {
    const parts: string[] = [];
    if (upgradeTalosVersion && upgradeTalosVersion !== currentTalos) {
      parts.push(`Talos ${currentTalos || '?'} → ${upgradeTalosVersion}`);
    }
    if (modulesDirty) {
      parts.push('module set change');
    }
    if (upgradeK8sVersion) {
      parts.push(`Kubernetes ${k8sCurrent || '?'} → ${upgradeK8sVersion}`);
    }
    return parts.length ? parts.join(' + ') : 'no change';
  }

  async function startClusterUpgrade() {
    const doingTalos =
      (upgradeTalosVersion && upgradeTalosVersion !== currentTalos) || modulesDirty;
    const doingK8s = !!upgradeK8sVersion;
    if (!doingTalos && !doingK8s) {
      notifyError('Select a new Talos version, change modules, or pick a Kubernetes version');
      return;
    }
    const msg = [
      `Start rolling upgrade: ${upgradeSummary()}?`,
      '',
      doingTalos
        ? '• Talos phase reboots nodes one at a time (workers first).'
        : '',
      doingK8s
        ? `• Kubernetes phase applies in place, no reboots, control-plane first.` +
          (k8sSupported.length > 1 ? '' : '')
        : '',
    ]
      .filter(Boolean)
      .join('\n');
    if (!confirm(msg)) return;
    busy = true;
    try {
      const res = (await client.post(`/clusters/${cid}/upgrade`, {
        talosVersion: doingTalos ? upgradeTalosVersion : undefined,
        k8sVersion: doingK8s ? upgradeK8sVersion : undefined,
        modules: modulesDirty ? [...clusterModules] : undefined,
        maxUnavailable: upgradeMaxUnavail,
        controlPlaneLast: upgradeCpLast,
      })) as { job?: { id: string }; k8sSteps?: string[] };
      const steps = res.k8sSteps || [];
      if (steps.length > 1) {
        success(`Upgrade job queued: ${res.job?.id} — k8s will run sequentially: ${steps.join(' → ')}`);
      } else {
        success(`Upgrade job queued${res.job?.id ? `: ${res.job.id}` : ''}`);
      }
      if (modulesDirty) {
        cluster.factoryModules = [...clusterModules];
        modulesDirty = false;
      }
      await loadUpgradeTargets();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to start cluster upgrade');
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
  const hasKubeconfig = $derived(!!(cluster && (cluster.hasKubeconfig || cluster.has_kubeconfig)));
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
        <Button variant="secondary" size="sm" title="Re-read node inventory and versions from the cluster's stored kubeconfig" onclick={refresh} disabled={busy}>Refresh from K8s</Button>
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
      <div class="info-item">
        <span class="info-label">Modules</span>
        <span class="info-value">
          {(cluster.factoryModules || []).length > 0
            ? (cluster.factoryModules || []).map(shortModuleName).join(', ')
            : 'default (none)'}
        </span>
      </div>
    </div>

    <details class="panel" open>
      <summary>Rolling upgrade</summary>
      <p class="sub">
        Choose a new Talos version and/or Kubernetes version, and adjust modules. Talos reboots
        nodes; Kubernetes applies in place (control-plane first, no reboots).
      </p>
      {#if targetsError}
        <p class="hint error">{targetsError}</p>
      {:else if targetsBusy && talosVersions.length === 0}
        <p class="hint">Loading upgrade targets…</p>
      {:else}
        {#if targetsNote}
          <p class="hint warning">{targetsNote}</p>
        {/if}
        <div class="upgrade-grid">
          <label>
            Talos version
            <select
              title="Target Talos version (image is built from this + the module set below)"
              bind:value={upgradeTalosVersion}
            >
              <option value={currentTalos}>{currentTalos || 'current'} (current)</option>
              {#each talosVersions as v (v)}
                {#if v !== currentTalos}
                  <option value={v}>{v}</option>
                {/if}
              {/each}
            </select>
          </label>
          <label>
            Kubernetes version
            <select
              title="Target Kubernetes version supported by this Talos build (in-place, no reboot)"
              bind:value={upgradeK8sVersion}
            >
              <option value="">{k8sCurrent || 'current'} (no change)</option>
              {#each k8sSupported as v (v)}
                <option value={v}>{v}</option>
              {/each}
            </select>
          </label>
          <label>
            Max unavailable
            <input type="number" title="How many nodes may be upgraded concurrently (Talos phase)" min="1" max="20" bind:value={upgradeMaxUnavail} />
          </label>
          <label class="check" style="align-self:end">
            <input type="checkbox" title="Upgrade workers before control-plane nodes for safety" bind:checked={upgradeCpLast} />
            Workers first
          </label>
        </div>

        {#if factoryError}
          <p class="hint error">{factoryError}</p>
        {:else if factoryBusy}
          <p class="hint">Loading module catalog…</p>
        {:else}
          <div class="module-picker">
            {#each factoryExtensions as f (f.name)}
              <label class="module-option" title={f.description || f.ref || ''}>
                <input type="checkbox" checked={clusterModules.has(f.name)} onchange={() => toggleClusterModule(f.name)} />
                <span class="mono">{shortModuleName(f.name)}</span>
                {#if f.author}<span class="hint"> · {f.author}</span>{/if}
              </label>
            {/each}
            {#if factoryExtensions.length === 0}
              <p class="hint">No modules returned for {upgradeTalosVersion || cluster.talosVersion || cluster.talos_version || 'this version'}.</p>
            {/if}
          </div>
          {#if clusterModules.size > 0}
            <p class="hint">
              Modules:
              {#each [...clusterModules].sort() as m (m)}
                <span class="module-chip mono">{shortModuleName(m)}</span>
              {/each}
            </p>
          {/if}
        {/if}

        <div class="form-actions">
          <Button
            variant="secondary"
            size="sm"
            title="Save the cluster's default module set without starting an upgrade"
            onclick={saveClusterModules}
            disabled={busy || !modulesDirty}
          >Save modules only</Button>
          <Button
            variant="primary"
            size="sm"
            title="Queue the rolling upgrade described above"
            onclick={startClusterUpgrade}
            disabled={busy || targetsBusy}
          >
            Start rolling upgrade{upgradeSummary() === 'no change' ? '' : ` — ${upgradeSummary()}`}
          </Button>
        </div>
      {/if}
    </details>

    <details class="panel">
      <summary>Cluster actions</summary>
      <div class="actions-stack">
        <div class="action-block">
          <h3>Workloads (Kubernetes)</h3>
          <p class="hint">
            Browse and manage pods, deployments, services, events, and nodes for this
            cluster. Uses this cluster's stored kubeconfig — it never leaves the server.
          </p>
          {#if !hasKubeconfig}
            <div class="empty-state">
              <p>No kubeconfig stored for this cluster</p>
              <p class="hint">Re-import the cluster with its kubeconfig to enable the explorer.</p>
            </div>
          {:else}
            <WorkloadsExplorer clusterId={cid ?? ''} />
          {/if}
        </div>
        <div class="action-block">
          <div class="upgrade-jobs">
            <h4>Upgrade jobs</h4>
            {#if upgradeJobsLoading && upgradeJobs.length === 0}
              <p class="hint">Loading…</p>
            {:else if upgradeJobs.length === 0}
              <p class="hint">No upgrade jobs for this cluster yet.</p>
            {:else}
              <table class="data-table">
                <thead>
                  <tr>
                    <th>Image</th>
                    <th>Status</th>
                    <th>Created</th>
                    <th>Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {#each upgradeJobs as job (job.id)}
                    <tr>
                      <td class="mono">{job.image || '—'}</td>
                      <td><span class="status-badge {job.status}">{job.status}</span></td>
                      <td>{job.createdAt || job.created_at ? new Date(job.createdAt || job.created_at || '').toLocaleString() : '—'}</td>
                      <td>
                        <div class="row-actions">
                          <Button variant="ghost" size="sm" title="View the full status and progress of this upgrade job" onclick={() => openUpgradeJob(job.id)}>Details</Button>
                          {#if job.status === 'pending' || job.status === 'running'}
                            <Button variant="danger" size="sm" title="Request cancellation of this running upgrade job" onclick={() => cancelUpgradeJob(job.id)}>Cancel</Button>
                          {/if}
                        </div>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            {/if}
            {#if upgradeJobDetail}
              <div class="job-detail">
                <div class="job-detail-header">
                  <h4>Job {upgradeJobDetail.job?.id || upgradeJobDetail.id}</h4>
                  <Button variant="ghost" size="sm" title="Close this job detail view" onclick={() => (upgradeJobDetail = null)}>Close</Button>
                </div>
                <pre class="job-json">{JSON.stringify(upgradeJobDetail, null, 2)}</pre>
              </div>
            {/if}
          </div>
        </div>
      </div>
    </details>

    <nav class="tabs">
      <button class:active={tab === 'machines'} title="Node inventory for this cluster" onclick={() => selectTab('machines')}>Machines</button>
      <button class:active={tab === 'config'} title="Cluster-wide Talos config patches" onclick={() => selectTab('config')}>Config</button>
      <button class:active={tab === 'backups'} title="Etcd snapshots and disaster recovery" onclick={() => selectTab('backups')}>Backups</button>
    </nav>

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
            <Button variant="ghost" size="sm" title="Validate patches against nodes without applying them" onclick={() => applyAll(true)} disabled={applying || patches.length === 0}>Dry-run</Button>
            <Button variant="secondary" size="sm" title="Apply all stored patches to every node in the cluster" onclick={() => applyAll(false)} disabled={applying || patches.length === 0}>
              {applying ? 'Applying…' : 'Apply to cluster'}
            </Button>
            <Button variant="primary" size="sm" title="Add a new config patch" onclick={() => (showEditor = !showEditor)}>
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
                <input type="text" title="Talos config path to patch, e.g. /machine/sysctls/net.ipv4.ip_forward" bind:value={newPatch.path} placeholder="/machine/sysctls/net.ipv4.ip_forward" />
              </div>
              <div class="form-group narrow">
                <label>Priority</label>
                <input type="number" title="Higher priority patches are applied later (override)" bind:value={newPatch.priority} />
              </div>
            </div>
            <div class="form-group">
              <label>Value (YAML)</label>
              <textarea title="The YAML value to set at the path above" bind:value={newPatch.value} rows="5" placeholder="true"></textarea>
            </div>
            <div class="editor-actions">
              <Button variant="primary" size="sm" title="Save this patch to the cluster" onclick={addPatch} disabled={saving}>
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
                  <Button variant="danger" size="sm" title="Delete this config patch" onclick={() => deletePatch(patch.id)}>Remove</Button>
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
          <Button variant="primary" size="sm" title="Take a new etcd snapshot of the cluster control plane" onclick={createBackup} disabled={creating}>
            {creating ? 'Creating…' : 'Create Backup'}
          </Button>
        </div>

        <section class="panel">
          <h3>Schedule</h3>
          <p class="hint">Automatic etcd snapshots when the cluster has a talosconfig. Scheduler checks about every 15 minutes.</p>
          <div class="inline-form">
            <label>
              Interval (hours, 0 = off)
              <input type="number" title="Hours between automatic snapshots; 0 disables the schedule" min="0" max="168" bind:value={scheduleHours} />
            </label>
            <label>
              Retention (ready backups to keep)
              <input type="number" title="Number of ready snapshots to retain before pruning" min="1" max="100" bind:value={retention} />
            </label>
            <Button variant="secondary" size="sm" title="Save the automatic backup schedule" onclick={saveSchedule} disabled={savingSchedule}>
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
          <select title="Control-plane node that will perform the etcd restore" bind:value={restoreMachineId}>
            <option value="">Auto (first control-plane / talosconfig endpoint)</option>
            {#each controlPlanes as m (m.id)}
              <option value={m.id}>{m.hostname || machineLabel(m)} — {m.address || 'no address'}</option>
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
                      <Button variant="ghost" size="sm" title="Download this etcd snapshot file" onclick={() => downloadBackup(backup)} disabled={backup.status !== 'ready'}>Download</Button>
                      <Button variant="secondary" size="sm" title="Restore the cluster control plane from this snapshot (disaster recovery)" onclick={() => restoreBackup(backup)} disabled={backup.status !== 'ready' || restoringId === backup.id}>
                        {restoringId === backup.id ? 'Restoring…' : 'Restore'}
                      </Button>
                      <Button variant="danger" size="sm" title="Permanently delete this snapshot" onclick={() => deleteBackup(backup)}>Delete</Button>
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

  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; margin: 0 0 0.75rem; }
  .hint.error { color: var(--tcs-error, #ef4444); }
  .hint.warning { color: var(--tcs-warning, #f59e0b); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
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
  .upgrade-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.75rem;
    margin-bottom: 0.75rem;
  }
  .upgrade-grid label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; }
  .upgrade-grid select {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    background: var(--tcs-surface);
    color: var(--tcs-text);
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
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
  .module-option .mono {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .module-option .hint {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .form-actions { display: flex; gap: 0.75rem; margin-top: 0.75rem; }

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
  .upgrade-jobs { margin-top: 1rem; }
  .upgrade-jobs h4 { margin: 0 0 0.5rem; font-size: 0.9rem; color: var(--tcs-text); }
  .job-detail {
    margin-top: 1rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem;
  }
  .job-detail-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.75rem; }
  .job-detail-header h4 { margin: 0; font-size: 0.9rem; }
  .job-json {
    margin: 0;
    padding: 0.75rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    font-size: 0.75rem;
    overflow: auto;
    max-height: 24rem;
  }

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
