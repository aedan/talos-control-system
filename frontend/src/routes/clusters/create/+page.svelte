<script lang="ts">
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  import type { FactoryExtension } from '$lib/api/types';

  // ── Step state ──
  let currentStep = $state(0);
  const steps = ['Cluster Details', 'Network', 'Machines', 'Provision'];

  // ── Step 1: Cluster Details ──
  let name = $state('');
  let endpoint = $state('');
  let controlPlaneVersion = $state('v1.36.3');
  let talosVersion = $state('v1.13.7');
  let clusterDomain = $state('cluster.local');
  let creating = $state(false);
  let clusterId = $state<string | null>(null);

  // ── Step 1 (cont.): Image Factory modules ──
  let factoryExtensions = $state<FactoryExtension[]>([]);
  let factoryBusy = $state(false);
  let factoryError = $state('');
  let selectedModules = $state<Set<string>>(new Set());
  function shortModuleName(full: string): string {
    const i = full.indexOf('/');
    return i >= 0 ? full.slice(i + 1) : full;
  }
  function toggleModule(name: string) {
    const next = new Set(selectedModules);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selectedModules = next;
  }
  async function loadFactoryModules() {
    factoryBusy = true;
    factoryError = '';
    try {
      const res = await client.get(`/factory/extensions?version=${encodeURIComponent(talosVersion)}`);
      factoryExtensions = ((res as { extensions: FactoryExtension[] }).extensions) || [];
    } catch (e: unknown) {
      factoryError = e instanceof Error ? e.message : 'Failed to load module catalog';
      factoryExtensions = [];
    } finally {
      factoryBusy = false;
    }
  }
  onMount(() => { void loadFactoryModules(); });

  // ── Step 2: Network ──
  let networkEnabled = $state(true);
  let bondName = $state('bond0');
  let bondInterfaces = $state('eno49, eno50');
  let bondMode = $state('802.3ad');
  let bondMiimon = $state(100);
  let bondLacpRate = $state('fast');
  let vlanName = $state('bond0.207');
  let vlanId = $state(207);
  let subnet = $state('162.242.191.0/26');
  let gateway = $state('162.242.191.65');
  let dnsServers = $state('172.24.16.254');
  let mtu = $state('');

  // ── Step 3: Machines ──
  let machines = $state<Array<{
    id: string;
    address: string;
    machineType: string;
    installDisk: string;
    status: string;
    hostname: string;
    macAddress: string;
    bmcAddress: string;
    hasBmc: boolean;
    lastPowerState: string;
    loadingDisks: boolean;
    installing: boolean;
    installProgress?: string;
  }>>([]);
  let newMachineAddress = $state('');
  let newMachineType = $state('controlplane');
  let newMachineMac = $state('');
  let newMachineBmc = $state('');
  let newMachineBmcUser = $state('');
  let newMachineBmcPass = $state('');
  let newMachineHostname = $state('');
  let addingMachine = $state(false);
  let importing = $state(false);
  let yamlImportText = $state('');

  // ── Step 4: Provision ──
  let generating = $state(false);
  let generated = $state<null | {
    id: string;
    controlplaneConfig?: string;
    workerConfig?: string;
    hasSecrets?: boolean;
  }>(null);
  let provisionJobId = $state<string | null>(null);
  let provisionBusy = $state(false);
  let jobStatus = $state('');
  let jobSteps = $state<string[]>([]);
  let currentMachineIdx = $state(0);
  let currentStepName = $state('');
  let jobPollTimer: number | null = null;

  // ── Actions ──
  async function handleCreate() {
    if (!name.trim()) {
      notifyError('Cluster name is required');
      return;
    }
    creating = true;
    try {
      const created = (await client.post('/clusters', {
        name: name.trim(),
        controlPlaneVersion,
        talosVersion,
        factoryModules: [...selectedModules],
      })) as { id?: string };
      clusterId = created?.id || null;
      success('Cluster record created.');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create cluster');
    } finally {
      creating = false;
    }
  }

  async function generateConfigs() {
    if (!name.trim() || !endpoint.trim()) {
      notifyError('Name and endpoint are required');
      return;
    }
    if (!clusterId) {
      await handleCreate();
    }
    generating = true;
    generated = null;
    try {
      const payload: any = {
        name: name.trim(),
        endpoint: endpoint.trim(),
        talosVersion,
        kubernetesVersion: controlPlaneVersion,
        clusterId: clusterId || null,
        clusterDomain: clusterDomain.trim() || 'cluster.local',
        wipe: true,
        certSans: [],
      };

      if (networkEnabled) {
        payload.network = {
          bondName,
          bondInterfaces: bondInterfaces.split(',').map(s => s.trim()).filter(Boolean),
          bondMode,
          bondMiimon,
          bondLacpRate,
          vlanName,
          vlanInterface: bondName,
          vlanId,
          subnet,
          gateway,
          dns: dnsServers.split(',').map(s => s.trim()).filter(Boolean),
          mtu: mtu ? parseInt(mtu, 10) : null,
        };
      }

      const art = (await client.post('/clusters/generate-config', payload)) as {
        id: string;
        controlplaneConfig?: string;
        workerConfig?: string;
        hasSecrets?: boolean;
      };
      generated = art;
      success('Machine configs generated with real PKI secrets.');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Config generation failed');
    } finally {
      generating = false;
    }
  }

  async function loadMachines() {
    if (!clusterId) return;
    try {
      const list = (await client.get(`/clusters/${clusterId}/machines`)) as Array<any>;
      machines = (list || []).map((m: any) => ({
        id: m.id,
        address: m.address || '',
        machineType: m.machineType || m.machine_type || 'worker',
        installDisk: m.installDisk || m.install_disk || '',
        status: m.status || 'pending',
        hostname: m.hostname || '',
        macAddress: m.macAddress || m.mac_address || '',
        bmcAddress: m.bmcAddress || m.bmc_address || '',
        hasBmc: m.hasBmc || !!m.bmcAddress,
        lastPowerState: m.lastPowerState || 'unknown',
        loadingDisks: false,
        installing: false,
      }));
    } catch {
      // machines may not exist yet
    }
  }

  async function addMachine() {
    if (!newMachineAddress.trim() && !newMachineMac.trim() && !newMachineBmc.trim()) {
      notifyError('Provide address, MAC, or BMC');
      return;
    }
    if (!clusterId) {
      notifyError('Create the cluster first');
      return;
    }
    addingMachine = true;
    try {
      await client.post('/machines', {
        systemUuid: `baremetal-${Date.now()}`,
        machineType: newMachineType,
        clusterId,
        address: newMachineAddress.trim() || undefined,
        macAddress: newMachineMac.trim() || undefined,
        bmcAddress: newMachineBmc.trim() || undefined,
        bmcUsername: newMachineBmcUser.trim() || undefined,
        bmcPassword: newMachineBmcPass || undefined,
        bmcType: 'auto',
        hostname: newMachineHostname.trim() || undefined,
      });
      success('Machine registered.');
      newMachineAddress = '';
      newMachineMac = '';
      newMachineBmc = '';
      newMachineBmcUser = '';
      newMachineBmcPass = '';
      newMachineHostname = '';
      await loadMachines();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to register machine');
    } finally {
      addingMachine = false;
    }
  }

  async function importYaml() {
    if (!yamlImportText.trim()) {
      notifyError('Paste YAML content first');
      return;
    }
    if (!clusterId) {
      notifyError('Create the cluster first');
      return;
    }
    importing = true;
    try {
      const result = (await client.post('/clusters/import', {
        yaml: yamlImportText.trim(),
        clusterId,
      })) as any;
      success(`Imported ${result?.machinesImported || '?'} machines.`);
      yamlImportText = '';
      await loadMachines();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Import failed');
    } finally {
      importing = false;
    }
  }

  async function startMetalProvision() {
    if (!clusterId || !generated) {
      notifyError('Generate configs first');
      return;
    }
    if (machines.length === 0) {
      notifyError('Add at least one machine');
      return;
    }
    provisionBusy = true;
    try {
      const job = (await client.post(`/clusters/${clusterId}/provision`, {
        machineIds: machines.map((m) => m.id),
        artifactId: generated.id,
        autoBootstrap: true,
      })) as { id: string };
      provisionJobId = job.id;
      jobStatus = 'pending';
      success('Provision job started');
      pollProvisionJob();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to start provision');
      provisionBusy = false;
    }
  }

  async function pollProvisionJob() {
    if (!provisionJobId) return;
    try {
      const job = (await client.get(`/provision-jobs/${provisionJobId}`)) as any;
      jobStatus = job.status || jobStatus;
      jobSteps = job.stepsLog || [];
      if (job.payload) {
        try {
          const p = typeof job.payload === 'string' ? JSON.parse(job.payload) : job.payload;
          currentMachineIdx = p.currentMachineIndex ?? 0;
          currentStepName = p.step || '';
        } catch { /* ignore */ }
      }
      // Refresh machines to show updated status
      await loadMachines();

      if (jobStatus === 'succeeded' || jobStatus === 'failed' || jobStatus === 'cancelled') {
        clearInterval(jobPollTimer || 0);
        jobPollTimer = null;
        provisionBusy = false;
        if (jobStatus === 'succeeded') {
          success('Provision job completed successfully!');
        } else if (jobStatus === 'failed') {
          notifyError(`Provision failed: ${job.error || 'unknown error'}`);
        }
        return;
      }
    } catch {
      // job may still be processing
    }
    if (!jobPollTimer) {
      jobPollTimer = window.setInterval(() => pollProvisionJob(), 5000);
    }
  }

  async function discoverDisks(index: number) {
    const m = machines[index];
    if (!m || !m.id) return;
    m.loadingDisks = true;
    try {
      const res = (await client.get(`/machines/${m.id}/disks`)) as { disks?: any[] };
      // We'll just set the install disk to the largest
      const disks = res.disks || [];
      if (disks.length > 0) {
        const best = disks.reduce((a, b) => (b.size || 0) > (a.size || 0) ? b : a);
        const devName = best.deviceName || best.name;
        if (devName) {
          await client.post(`/machines/${m.id}/install-disk`, { installDisk: devName });
          m.installDisk = devName;
          success(`Install disk set to ${devName}`);
        }
      } else {
        notifyError('No available disks found');
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to discover disks');
    } finally {
      m.loadingDisks = false;
    }
  }

  async function cancelJob() {
    if (!provisionJobId) return;
    try {
      await client.post(`/provision-jobs/${provisionJobId}/cancel`);
      success('Provision job cancelled');
      jobStatus = 'cancelled';
      if (jobPollTimer) {
        clearInterval(jobPollTimer);
        jobPollTimer = null;
      }
      provisionBusy = false;
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to cancel');
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

  function canProceed(step: number): boolean {
    switch (step) {
      case 0: return !!name.trim() && !!endpoint.trim();
      case 1: return true; // network is optional
      case 2: return machines.length > 0;
      case 3: return !!generated && machines.length > 0;
      default: return false;
    }
  }

  $effect(() => {
    // Load machines when clusterId changes
    if (clusterId) loadMachines();
  });

  // Cleanup on destroy
  $effect(() => {
    return () => {
      if (jobPollTimer) clearInterval(jobPollTimer);
    };
  });
</script>

<div class="provision-page">
  <h1>Provision bare metal cluster</h1>
  <p class="hint">
    Configure a new Talos cluster with full network control — bond, VLAN, DNS, and gateway — then provision machines via PXE + BMC.
  </p>

  <!-- ── Step navigation ── -->
  <div class="step-nav">
    {#each steps as step, i}
      <button
        class="step-btn {i === currentStep ? 'active' : ''} {i < currentStep ? 'done' : ''}"
        class:disabled={!canProceed(i)}
        title="Step {i + 1}: {step}"
        onclick={() => {
          if (i <= currentStep || canProceed(i)) currentStep = i;
        }}
      >
        <span class="step-num">{i + 1}</span>
        <span class="step-label">{step}</span>
      </button>
    {/each}
  </div>

  <!-- ── Step 1: Cluster Details ── -->
  {#if currentStep === 0}
    <div class="step-content">
      <h2>Cluster Details</h2>
      <p class="sub">Basic cluster identity and version selection.</p>

      <div class="form-grid">
        <div class="form-group">
          <label for="name">Cluster Name</label>
          <input id="name" type="text" title="Internal name for this cluster in TCS" bind:value={name} placeholder="kronos" required />
        </div>

        <div class="form-group">
          <label for="endpoint">API Endpoint</label>
          <input
            id="endpoint"
            type="text"
            title="Kubernetes API endpoint for the new cluster"
            bind:value={endpoint}
            placeholder="https://162.242.191.68:6443"
          />
        </div>

        <div class="form-group">
          <label for="domain">Cluster Domain</label>
          <input id="domain" type="text" title="Kubernetes cluster domain" bind:value={clusterDomain} placeholder="cluster.local" />
        </div>

        <div class="form-group">
          <label for="k8sVersion">Kubernetes Version</label>
          <select id="k8sVersion" title="Kubernetes version for the control plane" bind:value={controlPlaneVersion}>
            <option value="v1.36.3">v1.36.3</option>
            <option value="v1.35.1">v1.35.1</option>
            <option value="v1.34.0">v1.34.0</option>
            <option value="v1.31.0">v1.31.0</option>
            <option value="v1.30.4">v1.30.4</option>
            <option value="v1.29.8">v1.29.8</option>
          </select>
        </div>

        <div class="form-group">
          <label for="talosVersion">Talos Version</label>
          <select id="talosVersion" title="Talos Linux version to install on nodes" bind:value={talosVersion} onchange={() => loadFactoryModules()}>
            <option value="v1.13.7">v1.13.7</option>
            <option value="v1.12.0">v1.12.0</option>
            <option value="v1.11.0">v1.11.0</option>
            <option value="v1.10.0">v1.10.0</option>
            <option value="v1.9.0">v1.9.0</option>
          </select>
        </div>

        <div class="form-group">
          <label>System extensions (modules)</label>
          <p class="hint">
            Optional. Bake Talos system extensions into every node's image (e.g.
            <code>siderolabs/bnx2-bnx2x</code> for Broadcom 10G NICs). Nodes are upgraded to a
            factory image that includes them. Leave empty for the default image.
          </p>
          {#if factoryError}
            <p class="hint error">{factoryError}</p>
          {:else if factoryBusy}
            <p class="hint">Loading module catalog…</p>
          {:else}
            <div class="module-picker">
              {#each factoryExtensions as f (f.name)}
                <label class="module-option" title={f.description || f.ref || ''}>
                  <input type="checkbox" checked={selectedModules.has(f.name)} onchange={() => toggleModule(f.name)} />
                  <span class="mono">{shortModuleName(f.name)}</span>
                  {#if f.author}<span class="hint"> · {f.author}</span>{/if}
                </label>
              {/each}
              {#if factoryExtensions.length === 0}
                <p class="hint">No modules returned for {talosVersion}.</p>
              {/if}
            </div>
            {#if selectedModules.size > 0}
              <p class="hint">
                Selected:
                {#each [...selectedModules].sort() as m (m)}
                  <span class="module-chip mono">{shortModuleName(m)}</span>
                {/each}
              </p>
            {/if}
          {/if}
        </div>
      </div>

      <div class="form-actions">
        <Button variant="ghost" title="Cancel and go back" onclick={() => window.history.back()}>Cancel</Button>
        <Button variant="primary" title="Continue to network configuration" onclick={() => currentStep = 1} disabled={!canProceed(0)}>
          Next: Network →
        </Button>
      </div>
    </div>
  {/if}

  <!-- ── Step 2: Network ── -->
  {#if currentStep === 1}
    <div class="step-content">
      <h2>Network Configuration</h2>
      <p class="sub">
        Configure bonding, VLAN, and routing for Talos nodes. These interfaces will be created
        fresh by Talos — no MAAS network persistence.
      </p>

      <label class="toggle-row">
        <input type="checkbox" title="Enable a custom bond + VLAN network for the nodes" bind:checked={networkEnabled} />
        <span>Configure custom network (bond + VLAN)</span>
      </label>

      {#if networkEnabled}
        <div class="network-grid">
          <div class="network-section">
            <h3>Bond Interface</h3>
            <div class="form-grid">
              <div class="form-group">
                <label for="bondName">Bond Name</label>
                <input id="bondName" type="text" title="Name of the bond interface" bind:value={bondName} />
              </div>
              <div class="form-group">
                <label for="bondIfaces">Slave Interfaces</label>
                <input id="bondIfaces" type="text" title="Comma-separated NICs to bond, e.g. eno49, eno50" bind:value={bondInterfaces} placeholder="eno49, eno50" />
              </div>
              <div class="form-group">
                <label for="bondMode">Bond Mode</label>
                <select id="bondMode" title="Bonding mode (802.3ad = LACP)" bind:value={bondMode}>
                  <option value="802.3ad">802.3ad (LACP)</option>
                  <option value="active-backup">Active-Backup</option>
                  <option value="balance-rr">Balance-RR</option>
                </select>
              </div>
              <div class="form-group">
                <label for="bondMiimon">Miimon (ms)</label>
                <input id="bondMiimon" type="number" title="Bond link monitoring interval in milliseconds" bind:value={bondMiimon} />
              </div>
              <div class="form-group">
                <label for="lacpRate">LACP Rate</label>
                <select id="lacpRate" title="LACP negotiation rate" bind:value={bondLacpRate}>
                  <option value="fast">Fast (1s)</option>
                  <option value="slow">Slow (30s)</option>
                </select>
              </div>
            </div>
          </div>

          <div class="network-section">
            <h3>VLAN & Routing</h3>
            <div class="form-grid">
              <div class="form-group">
                <label for="vlanName">VLAN Interface</label>
                <input id="vlanName" type="text" title="Name of the VLAN interface, e.g. bond0.207" bind:value={vlanName} />
              </div>
              <div class="form-group">
                <label for="vlanId">VLAN ID</label>
                <input id="vlanId" type="number" title="VLAN ID to tag the bond interface with" bind:value={vlanId} />
              </div>
              <div class="form-group">
                <label for="subnet">Subnet</label>
                <input id="subnet" type="text" title="VLAN subnet in CIDR notation" bind:value={subnet} placeholder="162.242.191.0/26" />
              </div>
              <div class="form-group">
                <label for="gateway">Gateway</label>
                <input id="gateway" type="text" title="Default gateway on the VLAN subnet" bind:value={gateway} />
              </div>
              <div class="form-group">
                <label for="dns">DNS Servers</label>
                <input id="dns" type="text" title="Comma-separated DNS servers for the nodes" bind:value={dnsServers} placeholder="172.24.16.254" />
              </div>
              <div class="form-group">
                <label for="mtu">MTU (optional)</label>
                <input id="mtu" type="text" title="Optional MTU for the VLAN interface" bind:value={mtu} placeholder="1500" />
              </div>
            </div>
          </div>
        </div>
      {/if}

      <div class="form-actions">
        <Button variant="ghost" title="Go back to cluster details" onclick={() => currentStep = 0}>← Back</Button>
        <Button variant="primary" title="Continue to machine registration" onclick={() => currentStep = 2}>Next: Machines →</Button>
      </div>
    </div>
  {/if}

  <!-- ── Step 3: Machines ── -->
  {#if currentStep === 2}
    <div class="step-content">
      <h2>Register Machines</h2>
      <p class="sub">
        Add machines individually or import from YAML inventory. Each machine needs at least an address, MAC, or BMC.
      </p>

      <!-- Import YAML -->
      <div class="import-section">
        <h3>Import from YAML</h3>
        <textarea
          class="yaml-textarea"
          title="Machine inventory YAML to import"
          bind:value={yamlImportText}
          placeholder="# Paste your machine inventory YAML here"
          rows={6}
        ></textarea>
        <Button variant="secondary" size="sm" title="Import machines from the YAML above" onclick={importYaml} disabled={importing || !yamlImportText.trim()}>
          {importing ? 'Importing...' : 'Import YAML'}
        </Button>
      </div>

      <!-- Add individual machine -->
      <div class="add-section">
        <h3>Add Machine</h3>
        <div class="add-row">
          <input class="addr-input" type="text" title="Node hostname (optional)" bind:value={newMachineHostname} placeholder="Hostname (optional)" />
          <input class="addr-input" type="text" title="Node API address, e.g. 10.0.0.2" bind:value={newMachineAddress} placeholder="Address" />
          <input class="addr-input" type="text" title="Primary NIC MAC address" bind:value={newMachineMac} placeholder="MAC" />
          <input class="addr-input" type="text" title="Out-of-band BMC management IP" bind:value={newMachineBmc} placeholder="BMC IP" />
          <input class="addr-input" type="text" title="BMC management username" bind:value={newMachineBmcUser} placeholder="BMC user" />
          <input class="addr-input" type="password" title="BMC management password" bind:value={newMachineBmcPass} placeholder="BMC pass" />
          <select title="Node role: control-plane or worker" bind:value={newMachineType}>
            <option value="controlplane">Control</option>
            <option value="worker">Worker</option>
          </select>
          <Button variant="secondary" size="sm" title="Register this machine into the cluster" onclick={addMachine} disabled={addingMachine}>
            {addingMachine ? 'Adding...' : 'Add'}
          </Button>
        </div>
      </div>

      <!-- Machine list -->
      {#if machines.length > 0}
        <div class="machine-grid">
          {#each machines as m, i}
            <div class="machine-card status-{m.status}">
              <div class="machine-header">
                <span class="machine-role {m.machineType}">{m.machineType === 'controlplane' ? 'CP' : 'W'}</span>
                <div class="machine-info">
                  <span class="machine-hostname">{m.hostname || m.address || 'unnamed'}</span>
                  <span class="machine-detail">{m.macAddress}</span>
                </div>
                <span class="status-badge status-{m.status}">{m.status}</span>
              </div>
              <div class="machine-body">
                <div class="machine-meta">
                  {#if m.address}<span class="meta-item">📍 {m.address}</span>{/if}
                  {#if m.bmcAddress}<span class="meta-item">🔧 BMC: {m.bmcAddress}</span>{/if}
                  {#if m.lastPowerState !== 'unknown'}<span class="meta-item">⚡ {m.lastPowerState}</span>{/if}
                  {#if m.installDisk}<span class="meta-item">💾 {m.installDisk}</span>{/if}
                </div>
                <div class="machine-actions">
                  {#if !m.installDisk && m.status === 'pending'}
                    <Button variant="secondary" size="sm" title="Query the node for disks and select the largest as the install disk" onclick={() => discoverDisks(i)} disabled={m.loadingDisks}>
                      {m.loadingDisks ? 'Discovering...' : 'Discover disk'}
                    </Button>
                  {/if}
                </div>
              </div>
            </div>
          {/each}
        </div>
      {/if}

      <div class="form-actions">
        <Button variant="ghost" title="Go back to network configuration" onclick={() => currentStep = 1}>← Back</Button>
        <Button variant="primary" title="Continue to config generation and provisioning" onclick={() => currentStep = 3} disabled={!canProceed(2)}>
          Next: Provision →
        </Button>
      </div>
    </div>
  {/if}

  <!-- ── Step 4: Provision ── -->
  {#if currentStep === 3}
    <div class="step-content">
      <h2>Generate & Provision</h2>
      <p class="sub">
        Generate PKI + configs, then start the automated provision job.
      </p>

      <!-- Generate configs -->
      <div class="provision-section">
        <div class="config-summary">
          <div class="summary-row">
            <span class="label">Cluster:</span>
            <span class="value">{name}</span>
          </div>
          <div class="summary-row">
            <span class="label">Endpoint:</span>
            <span class="value">{endpoint}</span>
          </div>
          <div class="summary-row">
            <span class="label">Talos:</span>
            <span class="value">{talosVersion}</span>
          </div>
          <div class="summary-row">
            <span class="label">Kubernetes:</span>
            <span class="value">{controlPlaneVersion}</span>
          </div>
          <div class="summary-row">
            <span class="label">Machines:</span>
            <span class="value">{machines.filter(m => m.machineType === 'controlplane').length} CP + {machines.filter(m => m.machineType === 'worker').length} Worker</span>
          </div>
          <div class="summary-row">
            <span class="label">Network:</span>
            <span class="value">{networkEnabled ? `${bondName} + ${vlanName} (VLAN ${vlanId})` : 'Default (no custom network)'}</span>
          </div>
          <div class="summary-row">
            <span class="label">Install:</span>
            <span class="value text-error">Wipe: YES (fresh install)</span>
          </div>
        </div>

        <Button
          variant="primary"
          title="Generate cluster PKI and per-role Talos machine configs"
          onclick={generateConfigs}
          disabled={generating}
        >
          {#if generating}
            <Spinner />
          {:else}
            Generate PKI + Configs
          {/if}
        </Button>
      </div>

      {#if generated}
        <!-- Artifact result -->
        <div class="artifact-section">
          <div class="success-banner">
            ✅ Configs generated (artifact: <code>{generated.id}</code>)
          </div>
          <div class="dl-row">
            {#if generated.controlplaneConfig}
              <Button variant="secondary" size="sm" title="Download the control-plane Talos machine config" onclick={() => download(`${name}-controlplane.yaml`, generated!.controlplaneConfig!)}>
                Download CP config
              </Button>
            {/if}
            {#if generated.workerConfig}
              <Button variant="secondary" size="sm" title="Download the worker Talos machine config" onclick={() => download(`${name}-worker.yaml`, generated!.workerConfig!)}>
                Download Worker config
              </Button>
            {/if}
          </div>

          <!-- Start provision -->
          {#if !provisionJobId}
            <div class="provision-section">
              <p class="hint">Ready to provision {machines.length} machines. This will:</p>
              <ul class="provision-steps-list">
                <li>Set BMC boot mode to PXE</li>
                <li>Power cycle each machine</li>
                <li>Wait for Talos installer to appear</li>
                <li>Apply machine config and install Talos</li>
                <li>Bootstrap first control plane node</li>
                <li>Continue provisioning remaining machines</li>
              </ul>
              <Button variant="danger" title="Start the automated PXE/BMC provision job for all registered machines" onclick={startMetalProvision} disabled={provisionBusy}>
                {provisionBusy ? 'Starting...' : '🚀 Start Provision Job'}
              </Button>
            </div>
          {/if}
        </div>
      {/if}

      {#if provisionJobId}
        <!-- Job tracking -->
        <div class="job-tracking">
          <div class="job-header">
            <h3>Provision Job</h3>
            <div class="job-status status-{jobStatus}">
              {jobStatus}
            </div>
            {#if jobStatus !== 'succeeded' && jobStatus !== 'cancelled'}
              <Button variant="danger" size="sm" title="Cancel the running provision job" onclick={cancelJob}>Cancel</Button>
            {/if}
          </div>

          <div class="job-progress">
            <div class="progress-bar">
              <div class="progress-fill" style="width: {machines.length > 0 ? (currentMachineIdx / machines.length) * 100 : 0}%"></div>
            </div>
            <div class="progress-text">
              Machine {currentMachineIdx + 1} of {machines.length}
              {#if currentStepName} — {currentStepName}{/if}
            </div>
          </div>

          {#if jobSteps.length > 0}
            <div class="job-log">
              {#each jobSteps.slice(-20) as step}
                <div class="log-line">{step}</div>
              {/each}
            </div>
          {/if}

          {#if jobStatus === 'succeeded'}
            <div class="success-banner full">
              🎉 Cluster provisioned successfully!
              <div class="dl-row" style="margin-top: 1rem;">
                <Button variant="primary" title="Open the newly provisioned cluster" onclick={() => goto(clusterId ? `/clusters/${clusterId}` : '/')}>View Cluster</Button>
              </div>
            </div>
          {/if}

          {#if jobStatus === 'failed'}
            <div class="error-banner">
              Provision failed. Check logs above for details.
            </div>
          {/if}

          <!-- Machine status during provision -->
          {#if machines.length > 0}
            <div class="machine-grid">
              {#each machines as m, i}
                <div class="machine-card status-{m.status} {i === currentMachineIdx ? 'current' : ''}">
                  <div class="machine-header">
                    <span class="machine-role {m.machineType}">{m.machineType === 'controlplane' ? 'CP' : 'W'}</span>
                    <div class="machine-info">
                      <span class="machine-hostname">{m.hostname || m.address || `Machine ${i + 1}`}</span>
                    </div>
                    <span class="status-badge status-{m.status}">{m.status}</span>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      <div class="form-actions">
        <Button variant="ghost" title="Go back to machine registration" onclick={() => currentStep = 2}>← Back</Button>
        {#if generated}
          <Button variant="secondary" title="Return to the dashboard" onclick={() => goto('/')}>Back to Dashboard</Button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .provision-page h1 {
    margin: 0 0 0.5rem;
  }
  .hint {
    opacity: 0.85;
    margin-bottom: 1.5rem;
    max-width: 780px;
    line-height: 1.5;
    font-size: 0.9rem;
  }

  /* ── Step navigation ── */
  .step-nav {
    display: flex;
    gap: 0;
    margin-bottom: 2rem;
    border-bottom: 2px solid var(--tcs-border);
  }
  .step-btn {
    flex: 1;
    background: none;
    border: none;
    padding: 0.75rem 1rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    color: var(--tcs-text-muted);
    transition: all 0.2s;
    position: relative;
  }
  .step-btn:hover {
    color: var(--tcs-text);
    background: var(--tcs-surface);
  }
  .step-btn.active {
    color: var(--tcs-primary);
    font-weight: 600;
  }
  .step-btn.active::after {
    content: '';
    position: absolute;
    bottom: -2px;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--tcs-primary);
  }
  .step-btn.done {
    color: #22c55e;
  }
  .step-btn.disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .step-num {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.8rem;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
  }
  .step-btn.active .step-num {
    background: var(--tcs-primary);
    color: white;
    border-color: var(--tcs-primary);
  }
  .step-btn.done .step-num {
    background: #22c55e;
    color: white;
    border-color: #22c55e;
  }
  .step-label {
    font-size: 0.85rem;
  }

  /* ── Step content ── */
  .step-content {
    max-width: 900px;
  }
  .step-content h2 {
    margin: 0 0 0.25rem;
    font-size: 1.2rem;
  }
  .step-content .sub {
    color: var(--tcs-text-muted);
    margin-bottom: 1.5rem;
    font-size: 0.875rem;
  }

  /* ── Forms ── */
  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 1rem;
    margin-bottom: 1.5rem;
  }
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .form-group label {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
    font-weight: 500;
  }
  .form-group input,
  .form-group select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.55rem 0.7rem;
    color: var(--tcs-text);
    font-size: 0.875rem;
  }
  .form-group input:focus,
  .form-group select:focus {
    outline: none;
    border-color: var(--tcs-primary);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--tcs-primary) 20%, transparent);
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
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    font-size: 0.82rem;
    padding: 0.1rem 0;
  }
  .module-option input { flex: 0 0 auto; }
  .module-chip {
    display: inline-block;
    background: color-mix(in srgb, var(--tcs-primary) 15%, transparent);
    border: 1px solid var(--tcs-primary);
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.75rem;
    margin: 0.1rem 0.2rem 0.1rem 0;
  }
  .form-actions {
    display: flex;
    gap: 0.75rem;
    margin-top: 1.5rem;
  }

  /* ── Toggle ── */
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1.25rem;
    font-size: 0.9rem;
    cursor: pointer;
    color: var(--tcs-text);
  }

  /* ── Network ── */
  .network-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
    margin-bottom: 1.5rem;
  }
  .network-section {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem;
  }
  .network-section h3 {
    margin: 0 0 0.75rem;
    font-size: 0.95rem;
  }

  /* ── Import ── */
  .import-section {
    margin-bottom: 1.5rem;
  }
  .import-section h3 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
  }
  .yaml-textarea {
    width: 100%;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-text);
    font-family: monospace;
    font-size: 0.8rem;
    resize: vertical;
    margin-bottom: 0.5rem;
  }

  /* ── Add row ── */
  .add-section {
    margin-bottom: 1.5rem;
  }
  .add-section h3 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
  }
  .add-row {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .addr-input {
    flex: 1;
    min-width: 100px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
    color: var(--tcs-text);
    font-size: 0.8rem;
  }
  .add-row select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.45rem 0.5rem;
    color: var(--tcs-text);
    font-size: 0.8rem;
  }

  /* ── Machine grid ── */
  .machine-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 0.75rem;
    margin-top: 1rem;
  }
  .machine-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    transition: border-color 0.2s;
  }
  .machine-card.current {
    border-color: var(--tcs-primary);
    box-shadow: 0 0 0 1px var(--tcs-primary);
  }
  .machine-card.status-running { border-color: #22c55e33; }
  .machine-card.status-installing { border-color: #f59e0b33; }
  .machine-card.status-booting { border-color: #3b82f633; }
  .machine-card.status-failed { border-color: #ef444433; }
  .machine-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .machine-role {
    font-weight: 700;
    font-size: 0.7rem;
    padding: 0.15rem 0.4rem;
    border-radius: 3px;
    min-width: 28px;
    text-align: center;
  }
  .machine-role.controlplane {
    background: #3b82f622;
    color: #3b82f6;
  }
  .machine-role.worker {
    background: #8b5cf622;
    color: #8b5cf6;
  }
  .machine-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .machine-hostname {
    font-weight: 500;
    font-size: 0.85rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .machine-detail {
    font-size: 0.75rem;
    color: var(--tcs-text-muted);
  }
  .status-badge {
    font-size: 0.7rem;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    font-weight: 500;
    text-transform: uppercase;
  }
  .status-badge.status-pending { background: #6b728022; color: #9ca3af; }
  .status-badge.status-running { background: #22c55e22; color: #22c55e; }
  .status-badge.status-installing { background: #f59e0b22; color: #f59e0b; }
  .status-badge.status-booting { background: #3b82f622; color: #3b82f6; }
  .status-badge.status-failed { background: #ef444422; color: #ef4444; }
  .machine-body {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .machine-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .meta-item {
    font-size: 0.7rem;
    padding: 0.15rem 0.4rem;
    background: var(--tcs-background);
    border-radius: 3px;
    color: var(--tcs-text-muted);
  }
  .machine-actions {
    display: flex;
    gap: 0.35rem;
  }

  /* ── Provision step ── */
  .provision-section {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1.25rem;
    margin-bottom: 1.25rem;
  }
  .provision-steps-list {
    margin: 0.75rem 0;
    padding-left: 1.25rem;
    font-size: 0.85rem;
    color: var(--tcs-text-muted);
    line-height: 1.8;
  }

  /* ── Config summary ── */
  .config-summary {
    margin-bottom: 1rem;
  }
  .summary-row {
    display: flex;
    gap: 0.75rem;
    padding: 0.3rem 0;
    font-size: 0.85rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .summary-row:last-child { border-bottom: none; }
  .summary-row .label {
    color: var(--tcs-text-muted);
    min-width: 100px;
    font-weight: 500;
  }
  .summary-row .value {
    font-family: monospace;
    font-size: 0.8rem;
  }
  .text-error { color: #ef4444; }

  /* ── Artifact ── */
  .artifact-section {
    margin-bottom: 1.25rem;
  }
  .dl-row {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin: 0.75rem 0;
  }

  /* ── Job tracking ── */
  .job-tracking {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1.25rem;
    margin-bottom: 1.25rem;
  }
  .job-header {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .job-header h3 { margin: 0; font-size: 1rem; }
  .job-status {
    font-size: 0.8rem;
    padding: 0.2rem 0.6rem;
    border-radius: 4px;
    font-weight: 600;
    text-transform: uppercase;
  }
  .job-status.pending, .job-status.running { background: #3b82f622; color: #3b82f6; }
  .job-status.waiting_installer, .job-status.waiting_pxe { background: #f59e0b22; color: #f59e0b; }
  .job-status.installing { background: #8b5cf622; color: #8b5cf6; }
  .job-status.bootstrapping { background: #06b6d422; color: #06b6d4; }
  .job-status.succeeded { background: #22c55e22; color: #22c55e; }
  .job-status.failed { background: #ef444422; color: #ef4444; }
  .job-status.cancelled { background: #6b728022; color: #6b7280; }

  /* ── Progress ── */
  .job-progress {
    margin-bottom: 1rem;
  }
  .progress-bar {
    height: 6px;
    background: var(--tcs-background);
    border-radius: 3px;
    overflow: hidden;
    margin-bottom: 0.4rem;
  }
  .progress-fill {
    height: 100%;
    background: var(--tcs-primary);
    border-radius: 3px;
    transition: width 0.5s ease;
  }
  .progress-text {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }

  /* ── Job log ── */
  .job-log {
    background: var(--tcs-background);
    border-radius: 6px;
    padding: 0.75rem;
    max-height: 200px;
    overflow-y: auto;
    font-family: monospace;
    font-size: 0.75rem;
    margin-bottom: 1rem;
  }
  .log-line {
    padding: 0.15rem 0;
    border-bottom: 1px solid var(--tcs-border);
    color: var(--tcs-text-muted);
  }
  .log-line:last-child { border-bottom: none; }

  /* ── Banners ── */
  .success-banner {
    background: #22c55e15;
    border: 1px solid #22c55e33;
    color: #22c55e;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-size: 0.9rem;
    margin-bottom: 1rem;
  }
  .success-banner.full {
    text-align: center;
  }
  .error-banner {
    background: #ef444415;
    border: 1px solid #ef444433;
    color: #ef4444;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-size: 0.9rem;
    margin-bottom: 1rem;
  }

  @media (max-width: 640px) {
    .network-grid {
      grid-template-columns: 1fr;
    }
    .machine-grid {
      grid-template-columns: 1fr;
    }
    .step-btn .step-label {
      display: none;
    }
  }
</style>
