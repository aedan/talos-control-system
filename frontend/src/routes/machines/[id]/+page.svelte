<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { machineLabel, type Machine } from '$lib/api/types';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface ServiceRow {
    id: string;
    state: string;
    healthy: boolean;
    unknown: boolean;
  }

  let machine = $state<Machine | null>(null);
  let loading = $state(true);
  let error = $state('');
  let actionBusy = $state(false);
  let editAddress = $state('');
  let upgradeImage = $state('');
  let services = $state<ServiceRow[]>([]);
  let servicesError = $state('');
  let hostnameLive = $state('');
  let bmcStatus = $state<{
    configured?: boolean;
    powerState?: string;
    protocol?: string;
    error?: string;
  } | null>(null);
  let bmcAddress = $state('');
  let bmcUsername = $state('');
  let bmcPassword = $state('');
  let bmcType = $state('auto');
  let editMac = $state('');
  let editHostname = $state('');
  let editMachineType = $state('worker');
  let editInstallDisk = $state('');
  let editClusterId = $state('');
  let clusters = $state<Array<{ id: string; name: string }>>([]);

  // Machine config editor
  let configYaml = $state('');
  let configBusy = $state(false);
  let liveReachable = $state(false);
  let hasDesired = $state(false);
  let installImageHelper = $state('');
  let networkYamlHelper = $state('');
  let mountsYamlHelper = $state('');
  let applyReboot = $state(false);
  let applyMergeLive = $state(false);
  let isoUrl = $state('');
  let isoMedia = $state('CD');

  onMount(async () => {
    try {
      machine = (await client.get(`/machines/${$page.params.id}`)) as Machine;
      editAddress = machine.address || '';
      editMac = machine.macAddress || '';
      editHostname = machine.hostname || '';
      editMachineType = machine.machineType || 'worker';
      editInstallDisk = machine.installDisk || '';
      editClusterId = machine.clusterId || '';
      bmcAddress = machine.bmcAddress || '';
      bmcUsername = machine.bmcUsername || '';
      bmcType = machine.bmcType || 'auto';
      try {
        const cl = (await client.get('/clusters')) as Array<{ id: string; name: string }>;
        clusters = cl || [];
      } catch {
        clusters = [];
      }
      try {
        bmcStatus = (await client.get(`/machines/${$page.params.id}/bmc`)) as typeof bmcStatus;
      } catch {
        /* optional */
      }
      await loadDesiredConfig();
      void loadHostname();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine';
    } finally {
      loading = false;
    }
  });

  async function loadDesiredConfig() {
    try {
      const res = (await client.get(`/machines/${$page.params.id}/config`)) as {
        desiredConfig?: string | null;
        hasDesired?: boolean;
        liveReachable?: boolean;
      };
      hasDesired = !!res.hasDesired;
      liveReachable = !!res.liveReachable;
      if (res.desiredConfig) {
        configYaml = res.desiredConfig;
      }
    } catch {
      /* optional until cluster talosconfig set */
    }
  }

  async function loadLiveConfig() {
    configBusy = true;
    try {
      const res = (await client.get(`/machines/${$page.params.id}/config/live`)) as {
        configYaml: string;
      };
      configYaml = res.configYaml || '';
      liveReachable = true;
      success('Loaded live machine config from node');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load live config');
    } finally {
      configBusy = false;
    }
  }

  async function saveDesiredConfig() {
    if (!configYaml.trim()) {
      notifyError('Config YAML is empty');
      return;
    }
    configBusy = true;
    try {
      await client.put(`/machines/${$page.params.id}/config`, {
        configYaml,
      });
      hasDesired = true;
      success('Desired config saved (not applied to node yet)');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Save failed');
    } finally {
      configBusy = false;
    }
  }

  async function applyConfig(dryRun: boolean) {
    configBusy = true;
    try {
      const res = (await client.post(`/machines/${$page.params.id}/config/apply`, {
        configYaml: configYaml || undefined,
        dryRun,
        reboot: applyReboot,
        mergeWithLive: applyMergeLive,
      })) as { ok?: boolean; bytes?: number };
      success(
        dryRun
          ? `Dry-run OK (${res.bytes ?? 0} bytes)`
          : `Config applied${applyReboot ? ' (reboot requested)' : ''}`
      );
      if (!dryRun) hasDesired = true;
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Apply failed');
    } finally {
      configBusy = false;
    }
  }

  async function applyHelpers() {
    configBusy = true;
    try {
      const res = (await client.post(`/machines/${$page.params.id}/config/helpers`, {
        installImage: installImageHelper.trim() || undefined,
        networkYaml: networkYamlHelper.trim() || undefined,
        extraMountsYaml: mountsYamlHelper.trim() || undefined,
        hostname: editHostname.trim() || undefined,
        baseFromLive: true,
      })) as { desiredConfig?: string };
      if (res.desiredConfig) {
        configYaml = res.desiredConfig;
        hasDesired = true;
      }
      success('Helpers merged into desired config — review YAML then Apply');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Helper merge failed');
    } finally {
      configBusy = false;
    }
  }

  async function probeVersion() {
    actionBusy = true;
    try {
      const res = (await client.get(`/machines/${$page.params.id}/version`)) as {
        talosVersion: string;
      };
      if (machine) machine = { ...machine, talosVersion: res.talosVersion };
      success(`Talos version: ${res.talosVersion}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Version probe failed');
    } finally {
      actionBusy = false;
    }
  }

  async function loadHostname() {
    try {
      const res = (await client.get(`/machines/${$page.params.id}/hostname`)) as {
        hostname: string;
      };
      hostnameLive = res.hostname;
    } catch {
      /* optional */
    }
  }

  async function loadServices() {
    servicesError = '';
    try {
      const res = (await client.get(`/machines/${$page.params.id}/services`)) as {
        services: ServiceRow[];
      };
      services = res.services || [];
    } catch (e: unknown) {
      servicesError = e instanceof Error ? e.message : 'Failed to load services';
      services = [];
    }
  }

  async function reboot() {
    if (!confirm('Reboot this machine via Talos API?')) return;
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/reboot`, {});
      success('Reboot initiated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Reboot failed');
    } finally {
      actionBusy = false;
    }
  }

  async function upgrade() {
    if (!upgradeImage.trim()) {
      notifyError('Enter an installer image');
      return;
    }
    if (!confirm(`Upgrade with ${upgradeImage}?`)) return;
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/upgrade`, { image: upgradeImage.trim() });
      success('Upgrade initiated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Upgrade failed');
    } finally {
      actionBusy = false;
    }
  }

  async function saveAddress() {
    actionBusy = true;
    try {
      machine = (await client.put(`/machines/${$page.params.id}`, {
        address: editAddress.trim(),
      })) as Machine;
      success('Address updated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to update address');
    } finally {
      actionBusy = false;
    }
  }

  async function saveInventory() {
    actionBusy = true;
    try {
      const body: Record<string, unknown> = {
        hostname: editHostname.trim(),
        machineType: editMachineType,
        address: editAddress.trim(),
        macAddress: editMac.trim(),
        installDisk: editInstallDisk.trim(),
      };
      if (editClusterId.trim()) {
        body.clusterId = editClusterId.trim();
      } else {
        body.clearCluster = true;
      }
      machine = (await client.put(`/machines/${$page.params.id}`, body)) as Machine;
      success('Inventory saved');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save inventory');
    } finally {
      actionBusy = false;
    }
  }

  async function bootstrap() {
    if (!confirm('Bootstrap this control-plane node (initial etcd formation)?')) return;
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/bootstrap`, {});
      success('Bootstrap initiated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Bootstrap failed');
    } finally {
      actionBusy = false;
    }
  }

  async function saveBmc() {
    actionBusy = true;
    try {
      await client.put(`/machines/${$page.params.id}/bmc`, {
        bmcAddress: bmcAddress.trim(),
        bmcUsername: bmcUsername.trim(),
        bmcPassword: bmcPassword || undefined,
        bmcType,
      });
      if (editMac.trim()) {
        machine = (await client.put(`/machines/${$page.params.id}`, {
          macAddress: editMac.trim(),
        })) as Machine;
      }
      bmcPassword = '';
      bmcStatus = (await client.get(`/machines/${$page.params.id}/bmc`)) as typeof bmcStatus;
      success('BMC settings saved');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save BMC');
    } finally {
      actionBusy = false;
    }
  }

  async function powerAction(action: string) {
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/power`, { action });
      success(`Power ${action} sent`);
      bmcStatus = (await client.get(`/machines/${$page.params.id}/bmc`)) as typeof bmcStatus;
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : `Power ${action} failed`);
    } finally {
      actionBusy = false;
    }
  }

  async function bootPxe() {
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/boot-device`, {
        target: 'pxe',
        once: true,
      });
      success('Boot device set to PXE (once)');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Set boot PXE failed');
    } finally {
      actionBusy = false;
    }
  }

  async function mountIso() {
    if (!isoUrl.trim()) {
      notifyError('ISO URL is empty');
      return;
    }
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/mount-iso`, {
        isoUrl: isoUrl.trim(),
        media: isoMedia,
      });
      success(`ISO mounted to ${isoMedia}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Mount ISO failed');
    } finally {
      actionBusy = false;
    }
  }

  async function unmountIso() {
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/unmount-iso`, {
        media: isoMedia,
      });
      success(`ISO unmounted from ${isoMedia}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Unmount ISO failed');
    } finally {
      actionBusy = false;
    }
  }

  async function resetMachine() {
    if (
      !confirm(
        'DESTRUCTIVE: Reset/wipe this machine via Talos API? This is not recoverable from TCS alone.'
      )
    ) {
      return;
    }
    if (!confirm('Type intent confirmed: proceed with machine reset?')) return;
    actionBusy = true;
    try {
      await client.post(`/machines/${$page.params.id}/reset`, {
        confirm: true,
        graceful: true,
        reboot: true,
      });
      success('Machine reset initiated');
      machine = (await client.get(`/machines/${$page.params.id}`)) as Machine;
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Reset failed');
    } finally {
      actionBusy = false;
    }
  }
</script>

<div class="machine-detail">
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if machine}
    <div class="detail-header">
      <h1>{hostnameLive || machine.hostname || machineLabel(machine)}</h1>
      <div class="header-actions">
        <span class="status-badge">{machine.status}</span>
        <span class="type-badge">{machine.machineType}</span>
        <Button variant="secondary" size="sm" onclick={probeVersion} disabled={actionBusy}>Version</Button>
        <Button variant="secondary" size="sm" onclick={loadHostname} disabled={actionBusy}>Hostname</Button>
        <Button variant="secondary" size="sm" onclick={loadServices} disabled={actionBusy}>Services</Button>
        <Button variant="secondary" size="sm" onclick={bootstrap} disabled={actionBusy}>Bootstrap</Button>
        <Button variant="danger" size="sm" onclick={reboot} disabled={actionBusy}>Reboot</Button>
        <Button variant="danger" size="sm" onclick={resetMachine} disabled={actionBusy}>Reset</Button>
      </div>
    </div>

    <div class="info-grid">
      <div class="info-section">
        <h2>Inventory</h2>
        <div class="info-row"><span class="label">System UUID</span><span class="value mono">{machine.systemUuid}</span></div>
        <div class="info-row"><span class="label">Status</span><span class="value">{machine.status}</span></div>
        <div class="info-row"><span class="label">Talos</span><span class="value">{machine.talosVersion || '—'}</span></div>
        <div class="info-row"><span class="label">Created</span><span class="value">{machine.createdAt ? new Date(machine.createdAt).toLocaleString() : '—'}</span></div>
        <div class="form-row">
          <label>Hostname<input type="text" bind:value={editHostname} placeholder="cp-1" /></label>
        </div>
        <div class="form-row">
          <label>
            Role
            <select bind:value={editMachineType}>
              <option value="controlplane">controlplane</option>
              <option value="control-plane">control-plane</option>
              <option value="worker">worker</option>
            </select>
          </label>
        </div>
        <div class="form-row">
          <label>
            Cluster
            <select bind:value={editClusterId}>
              <option value="">— none —</option>
              {#each clusters as c (c.id)}
                <option value={c.id}>{c.name}</option>
              {/each}
            </select>
          </label>
        </div>
        <div class="form-row">
          <label>MAC<input type="text" bind:value={editMac} placeholder="aa:bb:cc:dd:ee:ff" /></label>
        </div>
        <div class="form-row">
          <label>Address<input type="text" bind:value={editAddress} placeholder="10.0.0.2 or host:50000" /></label>
        </div>
        <div class="form-row">
          <label>Install disk<input type="text" bind:value={editInstallDisk} placeholder="/dev/sda" /></label>
        </div>
        <Button variant="primary" size="sm" onclick={saveInventory} disabled={actionBusy}>Save inventory</Button>
      </div>

      <div class="info-section">
        <h2>Talos ops</h2>
        <div class="form-row">
          <label>
            Upgrade image
            <input type="text" bind:value={upgradeImage} placeholder="ghcr.io/siderolabs/installer:v1.8.0" />
          </label>
          <Button variant="secondary" size="sm" onclick={upgrade} disabled={actionBusy}>Upgrade</Button>
        </div>
        <p class="muted-hint">
          Per-node network, mounts, and install image: use the <strong>Machine config</strong>
          section below. Cluster-wide path patches remain under Cluster → Config.
        </p>
      </div>

      <div class="info-section">
        <h2>BMC / power</h2>
        <div class="info-row">
          <span class="label">Power</span>
          <span class="value">{bmcStatus?.powerState || machine.lastPowerState || 'unknown'}</span>
        </div>
        <div class="info-row">
          <span class="label">Protocol</span>
          <span class="value">{bmcStatus?.protocol || '—'}</span>
        </div>
        {#if bmcStatus?.error}
          <div class="error" style="margin:0.5rem 0">{bmcStatus.error}</div>
        {/if}
        <div class="form-row">
          <label>BMC address<input type="text" bind:value={bmcAddress} placeholder="192.168.1.100" /></label>
        </div>
        <div class="form-row">
          <label>BMC user<input type="text" bind:value={bmcUsername} /></label>
          <label>BMC password<input type="password" bind:value={bmcPassword} placeholder="••••••••" /></label>
        </div>
        <div class="form-row">
          <label>
            Type
            <select bind:value={bmcType}>
              <option value="auto">auto</option>
              <option value="redfish">redfish</option>
              <option value="ipmi">ipmi</option>
            </select>
          </label>
          <Button variant="secondary" size="sm" onclick={saveBmc} disabled={actionBusy}>Save BMC</Button>
        </div>
        <div class="header-actions" style="margin-top:0.5rem">
          <Button variant="secondary" size="sm" onclick={() => powerAction('on')} disabled={actionBusy}>On</Button>
          <Button variant="secondary" size="sm" onclick={() => powerAction('off')} disabled={actionBusy}>Off</Button>
          <Button variant="secondary" size="sm" onclick={() => powerAction('cycle')} disabled={actionBusy}>Cycle</Button>
          <Button variant="secondary" size="sm" onclick={bootPxe} disabled={actionBusy}>PXE once</Button>
        </div>
        <div class="form-row" style="margin-top:0.75rem; padding-top:0.75rem; border-top:1px solid var(--tcs-border)">
          <label>ISO URL
            <input type="text" bind:value={isoUrl} placeholder="http://localhost:6969/iso/talos-amd64.iso" />
          </label>
          <label>Media
            <select bind:value={isoMedia}>
              <option value="CD">CD</option>
              <option value="DVD">DVD</option>
              <option value="Floppy">Floppy</option>
            </select>
          </label>
          <Button variant="secondary" size="sm" onclick={mountIso} disabled={actionBusy}>Mount ISO</Button>
          <Button variant="secondary" size="sm" onclick={unmountIso} disabled={actionBusy}>Unmount</Button>
        </div>
      </div>
    </div>

    <section class="config-editor">
      <h2>Machine config</h2>
      <p class="muted-hint">
        Edit the full Talos machine config for this node (network, mounts, install image for
        factory/kernel modules, etc.). Save as desired copy, dry-run, then apply. Requires
        cluster talosconfig and a reachable machine address for live/apply.
        {#if hasDesired}<span class="badge">desired saved</span>{/if}
        {#if liveReachable}<span class="badge ok">node reachable</span>{:else}<span class="badge">live unknown</span>{/if}
      </p>

      <div class="helper-grid">
        <div class="info-section">
          <h3>Helpers (deep-merged into desired config)</h3>
          <label>
            Install image (factory / custom installer)
            <input
              type="text"
              bind:value={installImageHelper}
              placeholder="factory.talos.dev/metal-installer/<schematic-id>:v1.13.7"
            />
          </label>
          <label>
            Network YAML — deep-merged under machine.network (lists like
            <code>interfaces</code> are replaced, not appended)
            <textarea
              bind:value={networkYamlHelper}
              rows="6"
              spellcheck="false"
              placeholder="interfaces:
  - interface: eth0
    dhcp: true"
            ></textarea>
          </label>
          <label>
            Extra mounts (kubelet.extraMounts list)
            <textarea
              bind:value={mountsYamlHelper}
              rows="6"
              spellcheck="false"
              placeholder="- destination: /var/mnt/data
  type: bind
  source: /var/mnt/data
  options:
    - bind
    - rshared
    - rw"
            ></textarea>
          </label>
          <Button variant="secondary" size="sm" onclick={applyHelpers} disabled={configBusy}>
            Merge helpers into editor
          </Button>
        </div>
        <div class="info-section full">
          <div class="config-toolbar">
            <Button variant="secondary" size="sm" onclick={loadLiveConfig} disabled={configBusy}
              >Load live from node</Button
            >
            <Button variant="secondary" size="sm" onclick={loadDesiredConfig} disabled={configBusy}
              >Reload desired</Button
            >
            <Button variant="secondary" size="sm" onclick={saveDesiredConfig} disabled={configBusy}
              >Save desired</Button
            >
            <label class="check"
              ><input type="checkbox" bind:checked={applyMergeLive} /> Merge with live on apply</label
            >
            <label class="check"
              ><input type="checkbox" bind:checked={applyReboot} /> Reboot after apply</label
            >
            <Button variant="ghost" size="sm" onclick={() => applyConfig(true)} disabled={configBusy}
              >Dry-run</Button
            >
            <Button variant="primary" size="sm" onclick={() => applyConfig(false)} disabled={configBusy}
              >Apply to node</Button
            >
          </div>
          <textarea
            class="config-yaml"
            bind:value={configYaml}
            rows="22"
            spellcheck="false"
            placeholder="version: v1alpha1
machine:
  type: ...
  network: ...
  install:
    image: ...
    disk: ...
cluster:
  ..."
          ></textarea>
        </div>
      </div>
    </section>

    {#if servicesError}
      <div class="error">{servicesError}</div>
    {/if}
    {#if services.length > 0}
      <section class="services">
        <h2>Services</h2>
        <table class="data-table">
          <thead>
            <tr><th>ID</th><th>State</th><th>Health</th></tr>
          </thead>
          <tbody>
            {#each services as s (s.id)}
              <tr>
                <td class="mono">{s.id}</td>
                <td>{s.state}</td>
                <td>
                  {#if s.unknown}
                    <span class="health unk">unknown</span>
                  {:else if s.healthy}
                    <span class="health ok">healthy</span>
                  {:else}
                    <span class="health bad">unhealthy</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/if}
  {/if}
</div>

<style>
  .machine-detail h1 { margin: 0; }
  .detail-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin-bottom: 1.5rem;
  }
  .header-actions { display: flex; flex-wrap: wrap; gap: 0.4rem; align-items: center; }
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1rem;
    margin-bottom: 1.5rem;
  }
  .info-section {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
  }
  .info-section h2 { margin: 0 0 0.75rem; font-size: 1rem; }
  .info-row {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.35rem 0;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.9rem;
  }
  .label { color: var(--tcs-text-muted); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; word-break: break-all; }
  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: end;
    margin-bottom: 0.75rem;
  }
  label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; flex: 1; min-width: 12rem; }
  input {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
  }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
    margin-bottom: 1rem;
  }
  .muted-hint {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
    margin: 0.75rem 0 0;
    line-height: 1.4;
  }
  .config-editor {
    margin: 1.5rem 0;
  }
  .config-editor h2 { margin: 0 0 0.5rem; }
  .helper-grid {
    display: grid;
    grid-template-columns: minmax(240px, 1fr) 2fr;
    gap: 1rem;
  }
  @media (max-width: 900px) {
    .helper-grid { grid-template-columns: 1fr; }
  }
  .info-section.full { min-width: 0; }
  .config-toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin-bottom: 0.5rem;
  }
  .config-toolbar .check {
    flex-direction: row;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
  }
  .config-yaml, .info-section textarea {
    width: 100%;
    font-family: ui-monospace, monospace;
    font-size: 0.75rem;
    padding: 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
    box-sizing: border-box;
  }
  .info-section h3 { margin: 0 0 0.5rem; font-size: 0.95rem; }
  .badge {
    display: inline-block;
    margin-left: 0.35rem;
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
    font-size: 0.7rem;
  }
  .badge.ok { color: #4ade80; border-color: #4ade80; }
  .status-badge, .type-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
  }
  .services h2 { margin: 0 0 0.75rem; }
  .data-table { width: 100%; border-collapse: collapse; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .health.ok { color: var(--tcs-success, #22c55e); }
  .health.bad { color: var(--tcs-error, #ef4444); }
  .health.unk { color: var(--tcs-text-muted); }
</style>
