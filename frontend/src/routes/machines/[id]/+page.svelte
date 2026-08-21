<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { parseAllDocuments as yamlParseAll, stringify as yamlStringify } from 'yaml';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { machineLabel, type Machine } from '$lib/api/types';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  import {
    BOND_MODES,
    buildNetworkHelperYaml,
    newNetInterface,
    newNetVlan,
    parseNetworkIntoBuilder,
    type NetInterfaceBlock,
    type NetworkBuilderKeys,
  } from '$lib/networkBuilder';
  import { renderYamlDiff } from '$lib/diffView';

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
  let mountsYamlHelper = $state('');
  let netKeys = $state<NetworkBuilderKeys>({ interfaces: false, nameservers: false });
  let netInterfaces = $state<NetInterfaceBlock[]>([]);
  let netNameservers = $state<string[]>([]);
  let lastDiff = $state('');
  let showDiff = $state(false);
  let applyStatus = $state<{ kind: 'ok' | 'error'; text: string } | null>(null);
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
      if (!configYaml.trim()) {
        await loadLiveConfig(true);
      }
      populateHelpersFromConfig();
      void loadHostname();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine';
    } finally {
      loading = false;
    }
  });

  async function loadDesiredConfig() {
    const before = configYaml;
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
      return;
    }
    if (configYaml !== before) {
      lastDiff = renderYamlDiff(before, configYaml);
      showDiff = lastDiff !== '';
    }
  }

  async function loadLiveConfig(silent = false) {
    const before = configYaml;
    configBusy = true;
    try {
      const res = (await client.get(`/machines/${$page.params.id}/config/live`)) as {
        configYaml: string;
      };
      configYaml = res.configYaml || '';
      liveReachable = true;
      if (configYaml !== before) {
        lastDiff = renderYamlDiff(before, configYaml);
        showDiff = lastDiff !== '';
      }
      if (!silent) success('Loaded live machine config from node');
    } catch (e: unknown) {
      if (!silent) notifyError(e instanceof Error ? e.message : 'Failed to load live config');
    } finally {
      configBusy = false;
    }
  }

  function populateHelpersFromConfig(): { interfaces: number; nameservers: number } {
    if (!configYaml.trim()) return { interfaces: 0, nameservers: 0 };
    try {
      const parsed = parseNetworkIntoBuilder(configYaml);
      netInterfaces = parsed.interfaces;
      netNameservers = parsed.nameservers;
      if (parsed.interfaces.length > 0) netKeys.interfaces = true;
      if (parsed.nameservers.length > 0) netKeys.nameservers = true;

      const doc = yamlParseAll(configYaml)
        .map((d) => d.toJS({ maxAliasCount: 1000 }) as Record<string, any> | null)
        .find(Boolean) as Record<string, any> | null;
      const machine = doc?.machine as Record<string, any> | undefined;
      if (typeof machine?.install?.image === 'string') {
        installImageHelper = machine.install.image;
      }
      const mounts = machine?.kubelet?.extraMounts;
      if (Array.isArray(mounts) && mounts.length > 0) {
        mountsYamlHelper = yamlStringify(mounts);
      }
      return { interfaces: parsed.interfaces.length, nameservers: parsed.nameservers.length };
    } catch {
      /* keep helpers empty if the config cannot be parsed */
      return { interfaces: 0, nameservers: 0 };
    }
  }

  async function loadCurrentIntoBuilder() {
    if (!configYaml.trim()) {
      await loadLiveConfig(true);
    }
    if (!configYaml.trim()) {
      notifyError('No config loaded and live config is not reachable');
      return;
    }
    const { interfaces, nameservers } = populateHelpersFromConfig();
    if (interfaces === 0 && nameservers === 0) {
      notifyError('No interfaces or nameservers found in the loaded config');
    } else {
      success(`Loaded ${interfaces} interface(s) and ${nameservers} nameserver(s) into helpers`);
    }
  }

  function renderDiffHtml(patch: string): string {
    return patch
      .split('\n')
      .map((line) => {
        if (line.startsWith('@@')) return `<span class="diff-hunk">${line}</span>`;
        if (line.startsWith('+') && !line.startsWith('+++')) return `<span class="diff-add">${line}</span>`;
        if (line.startsWith('-') && !line.startsWith('---')) return `<span class="diff-del">${line}</span>`;
        return `<span class="diff-ctx">${line}</span>`;
      })
      .join('\n');
  }

  function addNetInterface() {
    netInterfaces.push(newNetInterface());
  }

  function removeNetInterface(id: string) {
    netInterfaces = netInterfaces.filter((b) => b.id !== id);
  }

  function addVlan(block: NetInterfaceBlock) {
    block.vlans.push(newNetVlan());
  }

  function removeVlan(block: NetInterfaceBlock, vlanId: string) {
    block.vlans = block.vlans.filter((v) => v.id !== vlanId);
  }

  function addNameServer() {
    netNameservers.push('');
  }

  function removeNameServer(idx: number) {
    netNameservers.splice(idx, 1);
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
    applyStatus = null;
    configBusy = true;
    try {
      const res = (await client.post(`/machines/${$page.params.id}/config/apply`, {
        configYaml: configYaml || undefined,
        dryRun,
        reboot: applyReboot,
        mergeWithLive: applyMergeLive,
      })) as { ok?: boolean; bytes?: number };
      const text = dryRun
        ? `Dry-run OK — config is valid (${res.bytes ?? 0} bytes)`
        : `Config applied to node${applyReboot ? ' (reboot requested)' : ''}`;
      applyStatus = { kind: 'ok', text };
      success(text);
      if (!dryRun) hasDesired = true;
    } catch (e: unknown) {
      const text = e instanceof Error ? e.message : 'Apply failed';
      applyStatus = { kind: 'error', text };
      notifyError(text);
    } finally {
      configBusy = false;
    }
  }

  async function applyHelpers() {
    const networkYaml = buildNetworkHelperYaml(
      { interfaces: netInterfaces, nameservers: netNameservers },
      netKeys
    );
    const before = configYaml;
    configBusy = true;
    try {
      const res = (await client.post(`/machines/${$page.params.id}/config/helpers`, {
        installImage: installImageHelper.trim() || undefined,
        networkYaml: networkYaml || undefined,
        extraMountsYaml: mountsYamlHelper.trim() || undefined,
        hostname: editHostname.trim() || undefined,
        baseFromLive: true,
      })) as { desiredConfig?: string };
      if (res.desiredConfig) {
        lastDiff = renderYamlDiff(before, res.desiredConfig);
        showDiff = lastDiff !== '';
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
        <Button variant="secondary" size="sm" title="Probe the node for its running Talos version" onclick={probeVersion} disabled={actionBusy}>Version</Button>
        <Button variant="secondary" size="sm" title="Fetch the node's live hostname from Talos" onclick={loadHostname} disabled={actionBusy}>Hostname</Button>
        <Button variant="secondary" size="sm" title="List the Talos services running on this node and their health" onclick={loadServices} disabled={actionBusy}>Services</Button>
        <Button variant="secondary" size="sm" title="Bootstrap this control-plane node (initial etcd formation)" onclick={bootstrap} disabled={actionBusy}>Bootstrap</Button>
        <Button variant="danger" size="sm" title="Reboot this machine via the Talos API" onclick={reboot} disabled={actionBusy}>Reboot</Button>
        <Button variant="danger" size="sm" title="DESTRUCTIVE: factory-reset and wipe this machine via Talos" onclick={resetMachine} disabled={actionBusy}>Reset</Button>
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
          <label>Hostname<input type="text" title="Node hostname as it appears in Talos" bind:value={editHostname} placeholder="cp-1" /></label>
        </div>
        <div class="form-row">
          <label>
            Role
            <select title="Node role: control-plane or worker" bind:value={editMachineType}>
              <option value="controlplane">controlplane</option>
              <option value="control-plane">control-plane</option>
              <option value="worker">worker</option>
            </select>
          </label>
        </div>
        <div class="form-row">
          <label>
            Cluster
            <select title="Cluster this machine belongs to" bind:value={editClusterId}>
              <option value="">— none —</option>
              {#each clusters as c (c.id)}
                <option value={c.id}>{c.name}</option>
              {/each}
            </select>
          </label>
        </div>
        <div class="form-row">
          <label>MAC<input type="text" title="Primary NIC MAC address (used for PXE/metal matching)" bind:value={editMac} placeholder="aa:bb:cc:dd:ee:ff" /></label>
        </div>
        <div class="form-row">
          <label>Address<input type="text" title="Node API endpoint, e.g. 10.0.0.2 or host:50000" bind:value={editAddress} placeholder="10.0.0.2 or host:50000" /></label>
        </div>
        <div class="form-row">
          <label>Install disk<input type="text" title="Block device Talos installs to, e.g. /dev/sda" bind:value={editInstallDisk} placeholder="/dev/sda" /></label>
        </div>
        <Button variant="primary" size="sm" title="Save inventory changes (hostname, role, cluster, MAC, address, disk)" onclick={saveInventory} disabled={actionBusy}>Save inventory</Button>
      </div>

      <div class="info-section">
        <h2>Talos ops</h2>
        <div class="form-row">
          <label>
            Upgrade image
            <input type="text" title="Talos installer image to upgrade this single node to" bind:value={upgradeImage} placeholder="ghcr.io/siderolabs/installer:v1.8.0" />
          </label>
          <Button variant="secondary" size="sm" title="Upgrade this node to the image above" onclick={upgrade} disabled={actionBusy}>Upgrade</Button>
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
          <label>BMC address<input type="text" title="Out-of-band management IP (IPMI/Redfish)" bind:value={bmcAddress} placeholder="192.168.1.100" /></label>
        </div>
        <div class="form-row">
          <label>BMC user<input type="text" title="BMC management username" bind:value={bmcUsername} /></label>
          <label>BMC password<input type="password" title="BMC management password (leave blank to keep current)" bind:value={bmcPassword} placeholder="••••••••" /></label>
        </div>
        <div class="form-row">
          <label>
            Type
            <select title="BMC protocol: auto-detect, Redfish, or IPMI" bind:value={bmcType}>
              <option value="auto">auto</option>
              <option value="redfish">redfish</option>
              <option value="ipmi">ipmi</option>
            </select>
          </label>
          <Button variant="secondary" size="sm" title="Save BMC connection settings" onclick={saveBmc} disabled={actionBusy}>Save BMC</Button>
        </div>
        <div class="header-actions" style="margin-top:0.5rem">
          <Button variant="secondary" size="sm" title="Power on the machine via BMC" onclick={() => powerAction('on')} disabled={actionBusy}>On</Button>
          <Button variant="secondary" size="sm" title="Power off the machine via BMC" onclick={() => powerAction('off')} disabled={actionBusy}>Off</Button>
          <Button variant="secondary" size="sm" title="Power-cycle the machine via BMC" onclick={() => powerAction('cycle')} disabled={actionBusy}>Cycle</Button>
          <Button variant="secondary" size="sm" title="Set the machine to boot from PXE once, then fall back to disk" onclick={bootPxe} disabled={actionBusy}>PXE once</Button>
        </div>
        <div class="form-row" style="margin-top:0.75rem; padding-top:0.75rem; border-top:1px solid var(--tcs-border)">
          <label>ISO URL
            <input type="text" title="URL of the ISO image to mount over the BMC virtual media" bind:value={isoUrl} placeholder="http://localhost:6969/iso/talos-amd64.iso" />
          </label>
          <label>Media
            <select title="Virtual media device type for the ISO" bind:value={isoMedia}>
              <option value="CD">CD</option>
              <option value="DVD">DVD</option>
              <option value="Floppy">Floppy</option>
            </select>
          </label>
          <Button variant="secondary" size="sm" title="Mount the ISO to the machine's virtual media" onclick={mountIso} disabled={actionBusy}>Mount ISO</Button>
          <Button variant="secondary" size="sm" title="Unmount the virtual media ISO" onclick={unmountIso} disabled={actionBusy}>Unmount</Button>
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
              title="Talos installer image to set under machine.install.image"
              bind:value={installImageHelper}
              placeholder="factory.talos.dev/metal-installer/<schematic-id>:v1.13.7"
            />
          </label>
          <label>
            Network blocks (deep-merged under machine.network)
            <div class="net-keys-row">
              <label class="check"
                ><input type="checkbox" title="Include the interface blocks in the merged network config" bind:checked={netKeys.interfaces} /> interfaces</label
              >
              <label class="check"
                ><input type="checkbox" title="Include the nameservers in the merged network config" bind:checked={netKeys.nameservers} /> nameservers</label
              >
              <Button variant="ghost" size="sm" title="Pull the current config's interfaces and nameservers into the helper fields" onclick={loadCurrentIntoBuilder} disabled={configBusy}
                >Load current values</Button
              >
            </div>
          </label>

          {#if netKeys.interfaces}
            <div class="block-list">
              {#each netInterfaces as block (block.id)}
                <div class="net-block">
                  <div class="net-block-row">
                    <input
                      type="text"
                      title="Interface name, e.g. eno1"
                      placeholder="interface (e.g. eno1)"
                      bind:value={block.interface}
                      class="mono"
                    />
                    <label class="check"
                      ><input type="checkbox" title="Use DHCP on this interface" bind:checked={block.dhcp} /> dhcp</label
                    >
                    <label class="check"
                      ><input type="checkbox" title="Ignore this interface in the generated config" bind:checked={block.ignore} /> ignore</label
                    >
                    <input
                      type="text"
                      title="MTU for this interface (optional)"
                      placeholder="mtu"
                      bind:value={block.mtu}
                      class="mono small"
                    />
                    <Button variant="ghost" size="sm" title="Remove this interface block" onclick={() => removeNetInterface(block.id)}
                      >remove</Button
                    >
                  </div>
                  <div class="net-block-col">
                    <div class="kv-row">
                      <span class="sub">addresses (CIDR)</span>
                      {#each block.addresses as _, i}
                        <div class="kv-row">
                          <input type="text" title="Interface address in CIDR form, e.g. 192.168.1.200/24" bind:value={block.addresses[i]} class="mono" />
                          <Button
                            variant="ghost"
                            size="sm"
                            title="Remove this interface address"
                            onclick={() => block.addresses.splice(i, 1)}
                            >–</Button
                          >
                        </div>
                      {/each}
                      <Button variant="ghost" size="sm" title="Add another address to this interface" onclick={() => block.addresses.push('')}
                        >+ address</Button
                      >
                    </div>
                    <div class="kv-row">
                      <span class="sub">routes (network / gateway / metric)</span>
                      {#each block.routes as _, i}
                        <div class="kv-row">
                            <input
                              type="text"
                              title="Destination network, e.g. 0.0.0.0/0"
                              placeholder="0.0.0.0/0"
                              bind:value={block.routes[i].network}
                              class="mono small"
                            />
                            <input
                              type="text"
                              title="Gateway IP for this route"
                              placeholder="192.168.1.2"
                              bind:value={block.routes[i].gateway}
                              class="mono small"
                            />
                            <input
                              type="text"
                              title="Route metric (optional)"
                              placeholder="metric"
                              bind:value={block.routes[i].metric}
                              class="mono small"
                            />
                            <Button
                              variant="ghost"
                              size="sm"
                              title="Remove this route"
                              onclick={() => block.routes.splice(i, 1)}
                              >–</Button
                            >
                        </div>
                      {/each}
                      <Button
                        variant="ghost"
                        size="sm"
                        title="Add another route to this interface"
                        onclick={() => block.routes.push({ network: '', gateway: '', metric: '' })}
                        >+ route</Button
                      >
                    </div>
                    <div class="kv-row">
                      <span class="sub">bond</span>
                      <select title="Bond mode for this interface (none = plain interface)" bind:value={block.bondMode}>
                        {#each BOND_MODES as mode}
                          <option value={mode}>{mode}</option>
                        {/each}
                      </select>
                      {#if block.bondMode !== 'none'}
                        <input
                          type="text"
                          title="Comma-separated bond member interfaces, e.g. eno49, eno50"
                          placeholder="members (e.g. eno49, eno50)"
                          bind:value={block.bondMembers}
                          class="mono"
                        />
                      {/if}
                    </div>
                    <div class="kv-row">
                      <span class="sub">vlans (nested on this interface)</span>
                      {#each block.vlans as vlan (vlan.id)}
                        <div class="vlan-block">
                          <div class="kv-row">
                            <span class="sub mono">vlan id</span>
                            <input
                              type="text"
                              title="VLAN ID, e.g. 207"
                              placeholder="207"
                              bind:value={vlan.vlanId}
                              class="mono small"
                            />
                            <input
                              type="text"
                              title="MTU for this VLAN (optional)"
                              placeholder="mtu"
                              bind:value={vlan.mtu}
                              class="mono small"
                            />
                            <label class="check"
                              ><input type="checkbox" title="Use DHCP on this VLAN" bind:checked={vlan.dhcp} /> dhcp</label
                            >
                            <Button variant="ghost" size="sm" title="Remove this VLAN" onclick={() => removeVlan(block, vlan.id)}
                              >remove</Button
                            >
                          </div>
                          <div class="kv-row">
                            <span class="sub">addresses (CIDR)</span>
                            {#each vlan.addresses as _, i}
                              <div class="kv-row">
                                <input type="text" title="VLAN address in CIDR form, e.g. 162.242.191.68/26" bind:value={vlan.addresses[i]} class="mono" />
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  title="Remove this VLAN address"
                                  onclick={() => vlan.addresses.splice(i, 1)}
                                  >–</Button
                                >
                              </div>
                            {/each}
                            <Button variant="ghost" size="sm" title="Add another address to this VLAN" onclick={() => vlan.addresses.push('')}
                              >+ address</Button
                            >
                          </div>
                          <div class="kv-row">
                            <span class="sub">routes (network / gateway / metric)</span>
                            {#each vlan.routes as _, i}
                              <div class="kv-row">
                                  <input
                                    type="text"
                                    title="Destination network, e.g. 0.0.0.0/0"
                                    placeholder="0.0.0.0/0"
                                    bind:value={vlan.routes[i].network}
                                    class="mono small"
                                  />
                                  <input
                                    type="text"
                                    title="Gateway IP for this route"
                                    placeholder="162.242.191.65"
                                    bind:value={vlan.routes[i].gateway}
                                    class="mono small"
                                  />
                                  <input
                                    type="text"
                                    title="Route metric (optional)"
                                    placeholder="metric"
                                    bind:value={vlan.routes[i].metric}
                                    class="mono small"
                                  />
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    title="Remove this route"
                                    onclick={() => vlan.routes.splice(i, 1)}
                                    >–</Button
                                  >
                              </div>
                            {/each}
                            <Button
                              variant="ghost"
                              size="sm"
                              title="Add another route to this VLAN"
                              onclick={() => vlan.routes.push({ network: '', gateway: '', metric: '' })}
                              >+ route</Button
                            >
                          </div>
                        </div>
                      {/each}
                      <Button variant="ghost" size="sm" title="Add a nested VLAN to this interface" onclick={() => addVlan(block)}
                        >+ vlan</Button
                      >
                    </div>
                  </div>
                </div>
              {/each}
              <Button variant="secondary" size="sm" title="Add a new interface block to the network helper" onclick={addNetInterface}
                >+ Add interface block</Button
              >
            </div>
          {/if}

          {#if netKeys.nameservers}
            <div class="block-list">
              <div class="net-block-col">
                <div class="kv-row">
                  <span class="sub">nameservers</span>
                  {#each netNameservers as _, i}
                    <div class="kv-row">
                      <input type="text" title="DNS server IP, e.g. 8.8.8.8" bind:value={netNameservers[i]} class="mono" />
                      <Button variant="ghost" size="sm" title="Remove this nameserver" onclick={() => removeNameServer(i)}
                        >–</Button
                      >
                    </div>
                  {/each}
                  <Button variant="ghost" size="sm" title="Add another nameserver" onclick={addNameServer}>+ nameserver</Button>
                </div>
              </div>
            </div>
          {/if}

          <details class="net-preview">
            <summary>Preview network YAML that will merge</summary>
            <pre>{buildNetworkHelperYaml({ interfaces: netInterfaces, nameservers: netNameservers }, netKeys) || '(no enabled keys)'}</pre>
          </details>
          <label>
            Extra mounts (kubelet.extraMounts list)
            <textarea
              title="YAML list of kubelet extraMounts entries to deep-merge into the config"
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
          <Button variant="secondary" size="sm" title="Merge the helper fields (image, network, mounts, hostname) into the config editor" onclick={applyHelpers} disabled={configBusy}>
            Merge helpers into editor
          </Button>
        </div>
        <div class="info-section full">
          <div class="config-toolbar">
            <Button variant="secondary" size="sm" title="Fetch the node's current live config and load it into the editor" onclick={() => loadLiveConfig()} disabled={configBusy}
              >Load live from node</Button
            >
            <Button variant="secondary" size="sm" title="Reload the saved desired config into the editor" onclick={loadDesiredConfig} disabled={configBusy}
              >Reload desired</Button
            >
            <Button variant="secondary" size="sm" title="Save the editor contents as the desired config (not applied yet)" onclick={saveDesiredConfig} disabled={configBusy}
              >Save desired</Button
            >
            <label class="check"
              ><input type="checkbox" title="Deep-merge the node's live config before applying" bind:checked={applyMergeLive} /> Merge with live on apply</label
            >
            <label class="check"
              ><input type="checkbox" title="Reboot the node after applying the config" bind:checked={applyReboot} /> Reboot after apply</label
            >
            <Button variant="ghost" size="sm" title="Validate the config against the node without applying it" onclick={() => applyConfig(true)} disabled={configBusy}
              >Dry-run</Button
            >
            <Button variant="primary" size="sm" title="Apply the editor config to the node" onclick={() => applyConfig(false)} disabled={configBusy}
              >Apply to node</Button
            >
          </div>
          {#if applyStatus}
            <div class="apply-status {applyStatus.kind}">
              <strong>{applyStatus.kind === 'ok' ? 'OK' : 'Error'}:</strong>
              <code>{applyStatus.text}</code>
            </div>
          {/if}
          <textarea
            class="config-yaml"
            title="Full Talos machine config YAML for this node"
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
          {#if showDiff && lastDiff}
            <details class="diff-view" open>
              <summary>What changed in the editor (vs before)</summary>
              <pre class="diff">{@html renderDiffHtml(lastDiff)}</pre>
            </details>
          {/if}
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
  .apply-status {
    margin: 0.5rem 0;
    padding: 0.5rem 0.6rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    font-size: 0.75rem;
    display: flex;
    gap: 0.4rem;
    align-items: baseline;
    max-height: 8rem;
    overflow: auto;
  }
  .apply-status code {
    font-family: ui-monospace, monospace;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--tcs-text);
  }
  .apply-status.ok {
    border-color: rgba(74, 222, 128, 0.4);
    background: rgba(74, 222, 128, 0.08);
    color: #4ade80;
  }
  .apply-status.error {
    border-color: rgba(248, 113, 113, 0.4);
    background: rgba(248, 113, 113, 0.08);
    color: #f87171;
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
  .net-keys-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: center;
    margin: 0.25rem 0 0.5rem;
  }
  .net-keys-row .check {
    flex-direction: row;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.8rem;
    margin: 0;
  }
  .block-list {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin: 0.25rem 0 0.75rem;
    border: 1px dashed var(--tcs-border);
    border-radius: 8px;
    padding: 0.6rem;
  }
  .net-block {
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 0.5rem 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    background: var(--tcs-background);
  }
  .net-block-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
  }
  .net-block-row .check {
    flex-direction: row;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    margin: 0;
  }
  .net-block-row input:not(.small) { flex: 1 1 10rem; min-width: 8rem; }
  .net-block-row input.small, .net-block-col input.small { width: 6rem; flex: 0 0 auto; }
  .net-block-col {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .kv-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    align-items: center;
  }
  .kv-row .sub {
    font-size: 0.7rem;
    color: var(--tcs-text-muted);
    min-width: 8rem;
  }
  .kv-row input { flex: 1 1 9rem; min-width: 7rem; font-size: 0.75rem; }
  .kv-row select {
    padding: 0.35rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
    font-size: 0.75rem;
  }
  .vlan-block {
    border: 1px dashed var(--tcs-border);
    border-radius: 6px;
    padding: 0.4rem 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    background: var(--tcs-surface);
  }
  .vlan-block .check {
    flex-direction: row;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    margin: 0;
  }
  .net-preview { margin: 0.4rem 0 0.75rem; font-size: 0.8rem; }
  .net-preview pre {
    margin: 0.35rem 0 0;
    padding: 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
    font-family: ui-monospace, monospace;
    font-size: 0.7rem;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .diff-view {
    margin: 0.6rem 0 0;
    font-size: 0.8rem;
  }
  .diff-view summary {
    cursor: pointer;
    margin-bottom: 0.35rem;
  }
  .diff-view pre {
    margin: 0;
    padding: 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    font-family: ui-monospace, monospace;
    font-size: 0.7rem;
    line-height: 1.45;
    overflow-x: auto;
  }
  .diff-view .diff-add { color: #4ade80; background: rgba(74, 222, 128, 0.08); display: block; }
  .diff-view .diff-del { color: #f87171; background: rgba(248, 113, 113, 0.08); display: block; }
  .diff-view .diff-hunk { color: #60a5fa; display: block; }
  .diff-view .diff-ctx { color: var(--tcs-text-muted); display: block; }
</style>
