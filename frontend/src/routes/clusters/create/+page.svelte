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
  let clusterId = $state<string | null>(null);

  let machines = $state<Array<{
    id: string;
    address: string;
    machineType: string;
    installDisk: string;
    status: string;
    disks: Array<{ deviceName: string; name: string; size: number; model: string }>;
    loadingDisks: boolean;
    installing: boolean;
  }>>([]);
  let newMachineAddress = '';
  let newMachineType = 'controlplane';
  let addingMachine = false;

  async function handleCreate() {
    if (!name.trim()) return;
    creating = true;
    try {
      const created = (await client.post('/clusters', {
        name: name.trim(),
        controlPlaneVersion,
        talosVersion,
      })) as { id?: string };
      clusterId = created?.id || null;
      success('Cluster inventory record created.');
      loadMachines();
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
      if (!clusterId && alsoInventory) {
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
        disks: [],
        loadingDisks: false,
        installing: false,
      }));
    } catch {
      // machines may not exist yet
    }
  }

  async function addMachine() {
    if (!newMachineAddress.trim()) return;
    addingMachine = true;
    try {
      await client.post('/machines', {
        systemUuid: `baremetal-${Date.now()}`,
        machineType: newMachineType,
        clusterId: clusterId,
        address: newMachineAddress.trim(),
      });
      success('Machine registered.');
      newMachineAddress = '';
      await loadMachines();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to register machine');
    } finally {
      addingMachine = false;
    }
  }

  async function discoverDisks(index: number) {
    const m = machines[index];
    if (!m || !m.id) return;
    m.loadingDisks = true;
    try {
      const res = (await client.get(`/machines/${m.id}/disks`)) as { disks?: any[] };
      m.disks = (res.disks || []).filter((d: any) => !d.systemDisk);
      if (m.disks.length === 0) {
        notifyError('No available disks found (all are system disks)');
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to discover disks');
    } finally {
      m.loadingDisks = false;
    }
  }

  async function selectDisk(index: number, device: string) {
    const m = machines[index];
    if (!m) return;
    try {
      await client.post(`/machines/${m.id}/install-disk`, {
        installDisk: device,
      });
      m.installDisk = device;
      success(`Install disk set to ${device}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to set install disk');
    }
  }

  async function installMachine(index: number) {
    const m = machines[index];
    if (!m || !generated) return;
    const config = m.machineType === 'controlplane'
      ? generated.controlplaneConfig
      : generated.workerConfig;
    if (!config) {
      notifyError('No config available for this machine type');
      return;
    }
    if (!m.installDisk) {
      notifyError('Select an install disk first');
      return;
    }
    m.installing = true;
    try {
      await client.post(`/machines/${m.id}/install`, {
        configYaml: config,
      });
      m.status = 'installing';
      success(`Install triggered on ${m.address}`);
      pollMachineStatus(index);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Install failed');
      m.installing = false;
    }
  }

  async function pollMachineStatus(index: number) {
    const m = machines[index];
    if (!m) return;
    const check = async () => {
      try {
        const info = (await client.get(`/machines/${m.id}`)) as any;
        m.status = info.status || m.status;
        if (m.status === 'running' || m.status === 'configuring') {
          m.installing = false;
          await loadMachines();
          return;
        }
      } catch {
        // machine may be rebooting
      }
      if (m.installing) setTimeout(check, 15000);
    };
    setTimeout(check, 15000);
  }

  async function bootstrapMachine(index: number) {
    const m = machines[index];
    if (!m) return;
    try {
      await client.post(`/machines/${m.id}/bootstrap`);
      success(`Bootstrap initiated on ${m.address}`);
      m.status = 'running';
      await loadMachines();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Bootstrap failed');
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

  function formatSize(bytes: number): string {
    if (bytes >= 1e12) return (bytes / 1e12).toFixed(1) + ' TB';
    if (bytes >= 1e9) return (bytes / 1e9).toFixed(1) + ' GB';
    if (bytes >= 1e6) return (bytes / 1e6).toFixed(1) + ' MB';
    return bytes + ' B';
  }
</script>

<div class="create-page">
  <h1>Provision bare metal</h1>
  <p class="hint">
    Register machines booted to the Talos installer, generate configs with real PKI secrets,
    then apply them to install Talos on disk.
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

    <div class="form-actions">
      <Button variant="ghost" type="button" onclick={() => window.history.back()}>Cancel</Button>
      <Button
        variant="secondary"
        type="button"
        onclick={handleCreate}
        disabled={creating || !!clusterId}
      >
        {creating ? 'Creating...' : clusterId ? 'Cluster created' : 'Create cluster record'}
      </Button>
      <Button
        variant="primary"
        type="button"
        onclick={generateConfigs}
        disabled={generating}
      >
        {generating ? 'Generating...' : 'Generate PKI + configs'}
      </Button>
    </div>
  </form>

  {#if clusterId}
    <section class="machine-section">
      <h2>Register machines</h2>
      <p class="muted">
        Add machines that are booted to the Talos installer (ISO, PXE, USB). TCS will reach
        them on port 50000.
      </p>

      <div class="add-row">
        <input
          class="addr-input"
          type="text"
          bind:value={newMachineAddress}
          placeholder="192.168.0.11"
        />
        <select bind:value={newMachineType}>
          <option value="controlplane">Control plane</option>
          <option value="worker">Worker</option>
        </select>
        <Button
          variant="secondary"
          size="sm"
          onclick={addMachine}
          disabled={addingMachine || !newMachineAddress.trim()}
        >
          {addingMachine ? 'Adding...' : 'Register'}
        </Button>
      </div>

      {#if machines.length > 0}
        <div class="machine-list">
          {#each machines as m, i}
            <div class="machine-card status-{m.status}">
              <div class="machine-header">
                <span class="machine-role">{m.machineType}</span>
                <span class="machine-addr">{m.address}</span>
                <span class="status-badge">{m.status}</span>
              </div>

              <div class="machine-actions">
                {#if !m.disks.length && m.status === 'pending'}
                  <Button
                    variant="secondary"
                    size="sm"
                    onclick={() => discoverDisks(i)}
                    disabled={m.loadingDisks}
                  >
                    {m.loadingDisks ? 'Discovering...' : 'Discover disks'}
                  </Button>
                {/if}

                {#if m.disks.length > 0 && !m.installDisk}
                  <select
                    class="disk-select"
                    onchange={(e) => selectDisk(i, (e.target as HTMLSelectElement).value)}
                  >
                    <option value="">Select install disk</option>
                    {#each m.disks as disk}
                      <option value={disk.deviceName}>
                        {disk.deviceName} ({formatSize(disk.size)}{disk.model ? ' - ' + disk.model : ''})
                      </option>
                    {/each}
                  </select>
                {/if}

                {#if m.installDisk}
                  <span class="disk-badge">Disk: {m.installDisk}</span>
                {/if}

                {#if generated && m.installDisk && m.status === 'pending'}
                  <Button
                    variant="primary"
                    size="sm"
                    onclick={() => installMachine(i)}
                    disabled={m.installing}
                  >
                    {m.installing ? 'Installing...' : 'Install Talos'}
                  </Button>
                {/if}

                {#if m.status === 'configuring' && m.machineType === 'controlplane'}
                  <Button
                    variant="primary"
                    size="sm"
                    onclick={() => bootstrapMachine(i)}
                  >
                    Bootstrap
                  </Button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

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
        <pre class="preview">{generated.controlplaneConfig.slice(0, 1200)}{#if generated.controlplaneConfig.length > 1200}...{/if}</pre>
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
  .form-actions {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .machine-section {
    margin-top: 2rem;
    max-width: 720px;
  }
  .machine-section h2 {
    margin: 0 0 0.4rem;
    font-size: 1.05rem;
  }
  .add-row {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }
  .addr-input {
    flex: 1;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.7rem;
    color: var(--tcs-text);
    font-size: 0.875rem;
  }
  .add-row select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.6rem;
    color: var(--tcs-text);
    font-size: 0.875rem;
  }
  .machine-list {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin-top: 1rem;
  }
  .machine-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
  }
  .machine-card.status-running {
    border-color: #22c55e44;
  }
  .machine-card.status-installing {
    border-color: #f59e0b44;
  }
  .machine-card.status-booting {
    border-color: #3b82f644;
  }
  .machine-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }
  .machine-role {
    font-weight: 600;
    font-size: 0.85rem;
    text-transform: uppercase;
    min-width: 80px;
  }
  .machine-addr {
    flex: 1;
    font-size: 0.9rem;
    color: var(--tcs-text-muted);
  }
  .status-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: var(--tcs-background);
    color: var(--tcs-text-muted);
    text-transform: uppercase;
  }
  .machine-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .disk-select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    color: var(--tcs-text);
    font-size: 0.8rem;
  }
  .disk-badge {
    font-size: 0.8rem;
    padding: 0.3rem 0.6rem;
    border-radius: 4px;
    background: var(--tcs-background);
    color: var(--tcs-text-muted);
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