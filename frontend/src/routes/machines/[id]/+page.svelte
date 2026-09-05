<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { parseAllDocuments as yamlParseAll, stringify as yamlStringify } from 'yaml';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { machineLabel, type Machine, type MachineVersions, type MachineExtension, type FactoryExtension } from '$lib/api/types';
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
  import { openConsoleSession, closeConsoleSession, getIdracCredentials, openSolSession, type ConsoleSession, type SolHandle } from '$lib/api/iloConsole';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';

  let machine = $state<Machine | null>(null);
  let loading = $state(true);
  let error = $state('');
  let actionBusy = $state(false);
  let editAddress = $state('');
  // OOB console overlay state
  let consoleOpen = $state(false);
  let consoleBusy = $state(false);
  let consoleMode = $state<'ilo' | 'sol' | 'none'>('none');
  let consoleEmbed = $state('');
  let consoleSid = $state('');
  let consoleIdracUrl = $state('');
  let idracCredsOpen = $state(false);
  let idracCreds = $state<{ username: string; password: string; idracUrl: string } | null>(null);
  let idracCredsLoading = $state(false);
  let idracCredsRevealed = $state(false);
  let consoleError = $state('');
  let versions = $state<MachineVersions | null>(null);
  let extensions = $state<MachineExtension[]>([]);
  let extBusy = $state(false);
  let extError = $state('');
  // Module management (Image Factory)
  let factoryVersions = $state<string[]>([]);
  let factoryExtensions = $state<FactoryExtension[]>([]);
  let factoryBusy = $state(false);
  let factoryError = $state('');
  let selectedVersion = $state('');
  let effectiveModules = $state<string[]>([]);
  let editModules = $state<Set<string>>(new Set());
  let modulesDirty = $state(false);
  let applyBusy = $state(false);
  let applyMessage = $state('');
  let clusterModules = $state<string[]>([]);
  let clusterModulesBusy = $state(false);
  // Node-level deltas against the cluster default module set.
  let adds = $state<string[]>([]);
  let removes = $state<string[]>([]);
  let overridesBusy = $state(false);
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
      // Normalize legacy role spellings to the canonical backend value
      // ('controlplane'), since the Role select only offers controlplane/worker.
      const rawType = (machine.machineType || 'worker').toLowerCase();
      editMachineType = rawType === 'control-plane' || rawType === 'control_plane' || rawType === 'cp'
        ? 'controlplane'
        : (rawType === 'worker' ? 'worker' : machine.machineType || 'worker');
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
      void loadImageAndModules(true);
      void loadClusterModules();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine';
    } finally {
      loading = false;
    }
  });

  // Human-friendly role label for the header badge (canonical value is
  // 'controlplane'; legacy rows may store 'control-plane'/'control_plane').
  function roleLabel(t: string | undefined): string {
    const s = (t || '').toLowerCase();
    if (s === 'controlplane' || s === 'control-plane' || s === 'control_plane' || s === 'cp') {
      return 'control-plane';
    }
    return t || '—';
  }

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

  // ---- OOB console ----

  let solTerm: Terminal | null = null;
  let solFit: FitAddon | null = null;
  let solHandle: SolHandle | null = null;
  let solEl: HTMLDivElement | null = null;

  // Svelte action: mount the SOL terminal into the div when it appears.
  function solContainer(node: HTMLDivElement) {
    solEl = node;
    if (consoleMode === 'sol') {
      // Defer so the overlay layout is painted before fit().
      requestAnimationFrame(() => {
        startSol();
        window.addEventListener('resize', onSolResize);
      });
    }
    return {
      destroy() {
        window.removeEventListener('resize', onSolResize);
        stopSol();
        solEl = null;
      },
    };
  }

  function onSolResize() {
    solFit?.fit();
  }

  async function openConsole() {
    if (!machine) return;
    consoleBusy = true;
    consoleError = '';
    consoleMode = 'none';
    try {
      const res: ConsoleSession = await openConsoleSession($page.params.id!);
      if (!res.ok && res.mode === 'none') {
        consoleError = res.error || 'Console unavailable';
        return;
      }
      consoleMode = res.mode;
      consoleSid = res.sessionId || '';
      consoleEmbed = res.embedUrl || '';
      consoleIdracUrl = res.idracConsoleUrl || '';
      consoleOpen = true;
      if (res.mode === 'sol' && res.idracConsoleUrl) {
        // Dell: open the iDRAC in a new tab. `login.html?console` deep-links
        // straight into the Virtual Console (video) after login — the user's
        // browser password manager autofills the form. SOL stays inline as a
        // fallback if the iDRAC tab isn't usable.
        window.open(res.idracConsoleUrl, '_blank', 'noopener');
      }
      if (res.mode === 'sol') {
        // SOL output banner (informational, not an error). The terminal is
        // mounted by the solContainer action when the div appears.
        if (res.error) consoleError = res.error;
      }
    } catch (e: unknown) {
      consoleError = e instanceof Error ? e.message : 'Failed to open console';
    } finally {
      consoleBusy = false;
    }
  }

  function closeConsole() {
    stopSol();
    if (consoleMode === 'ilo' && consoleSid) {
      void closeConsoleSession($page.params.id!, consoleSid);
    }
    consoleOpen = false;
    consoleMode = 'none';
    consoleEmbed = '';
    consoleSid = '';
    consoleIdracUrl = '';
    idracCredsOpen = false;
    idracCreds = null;
    idracCredsRevealed = false;
    consoleError = '';
  }

  // "Show iDRAC login": reveal the stored iDRAC creds for this Dell machine.
  async function showIdracCreds() {
    if (idracCredsLoading) return;
    idracCredsOpen = true;
    idracCredsLoading = true;
    idracCredsRevealed = false;
    idracCreds = null;
    try {
      const c = await getIdracCredentials($page.params.id!);
      idracCreds = { username: c.username, password: c.password, idracUrl: c.idracUrl };
    } catch (e: unknown) {
      consoleError = e instanceof Error ? e.message : 'Failed to load iDRAC credentials';
    } finally {
      idracCredsLoading = false;
    }
  }

  function closeIdracCreds() {
    idracCredsOpen = false;
    idracCreds = null;
    idracCredsRevealed = false;
  }

  async function copyCred(field: 'username' | 'password') {
    if (!idracCreds) return;
    try {
      await navigator.clipboard.writeText(idracCreds[field]);
    } catch {
      // clipboard unavailable; the value is visible for manual copy
    }
  }

  function startSol() {
    stopSol();
    if (!solEl) return;
    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      theme: { background: '#0b0e14', foreground: '#d7dae0' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(solEl);
    fit.fit();
    solTerm = term;
    solFit = fit;

    term.onData((data) => solHandle?.send(data));
    const handle = openSolSession(
      $page.params.id!,
      (bytes) => term.write(bytes),
      () => {
        if (solTerm === term) {
          term.write('\r\n\x1b[90m[SOL closed]\x1b[0m\r\n');
        }
      },
    );
    solHandle = handle;
    term.focus();
  }

  function stopSol() {
    solHandle?.close();
    solHandle = null;
    solTerm?.dispose();
    solTerm = null;
    solFit = null;
  }

  // Factory extension names are "org/module" (e.g. siderolabs/bnx2-bnx2x); the
  // node reports loaded extensions by the short "module" name. Normalize both.
  function shortModuleName(full: string): string {
    const i = full.indexOf('/');
    return i >= 0 ? full.slice(i + 1) : full;
  }

  async function loadImageAndModules(silent = false) {    extError = '';
    extBusy = true;
    try {
      const [vRes, eRes, mRes] = await Promise.all([
        client.get(`/machines/${$page.params.id}/versions`),
        client.get(`/machines/${$page.params.id}/extensions`),
        client.get(`/machines/${$page.params.id}/modules`),
      ]);
      versions = (vRes as MachineVersions) || null;
      extensions = ((eRes as { extensions: MachineExtension[] }).extensions) || [];
      effectiveModules = ((mRes as { modules: string[] }).modules) || [];
      // Prefill the picker from the effective modules.
      editModules = new Set(effectiveModules);
      modulesDirty = false;
      // Load the factory module catalog for the running version (best-effort).
      const ver = versions?.version || machine?.talosVersion || '';
      if (ver) void loadFactoryCatalog(ver, silent);
    } catch (e: unknown) {
      extError = e instanceof Error ? e.message : 'Failed to probe image & modules';
      versions = null;
      extensions = [];
    } finally {
      extBusy = false;
      if (!silent) success('Probed image & modules');
    }
  }

  async function loadFactoryCatalog(version: string, silent = false) {
    factoryError = '';
    if (!factoryBusy) factoryBusy = true;
    try {
      // Version list (to allow switching) + the official extensions for the chosen version.
      const vRes = await client.get(`/factory/versions`);
      factoryVersions = ((vRes as { versions: string[] }).versions) || [];
      if (!selectedVersion) selectedVersion = version;
      const eRes = await client.get(`/factory/extensions?version=${encodeURIComponent(selectedVersion)}`);
      factoryExtensions = ((eRes as { extensions: FactoryExtension[] }).extensions) || [];
    } catch (e: unknown) {
      factoryError = e instanceof Error ? e.message : 'Failed to load Image Factory catalog';
      factoryExtensions = [];
      factoryVersions = [];
    } finally {
      factoryBusy = false;
      if (!silent) { /* silent catalog load */ }
    }
  }

  function toggleModule(name: string) {
    const next = new Set(editModules);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    editModules = next;
    modulesDirty = [...editModules].sort().join('|') !== [...effectiveModules].sort().join('|');
  }

  // ── node-level deltas ─────────────────────────────────────────────────────
  function isClusterDefault(name: string): boolean {
    return clusterModules.includes(name);
  }
  function toggleAdd(name: string) {
    const next = new Set(adds);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    adds = [...next].sort();
    removes = removes.filter((r) => r !== name);
  }
  function toggleRemove(name: string) {
    const next = new Set(removes);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    removes = [...next].sort();
    adds = adds.filter((a) => a !== name);
  }
  async function saveOverrides() {
    if (overridesBusy) return;
    overridesBusy = true;
    try {
      const res = (await client.put(`/machines/${$page.params.id}/module-overrides`, {
        adds,
        removes,
      })) as { effective?: string[] };
      if (res.effective) {
        effectiveModules = res.effective;
        editModules = new Set(effectiveModules);
        modulesDirty = false;
      }
      success('Module overrides saved');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save overrides');
    } finally {
      overridesBusy = false;
    }
  }

  async function saveModules() {
    if (applyBusy) return;
    try {
      await client.put(`/machines/${$page.params.id}/modules`, { modules: [...editModules] });
      effectiveModules = [...editModules];
      modulesDirty = false;
      success('Modules saved');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save modules');
    }
  }

  async function applyModules() {
    if (applyBusy) return;
    const mods = [...editModules];
    if (mods.length === 0) {
      notifyError('Select at least one module to apply');
      return;
    }
    const ok = confirm(
      `Apply modules [${mods.join(', ')}] to ${machine?.hostname || 'this machine'}?\n\n` +
        `This upgrades the node to a factory image that bundles those modules and REBOOTS it. ` +
        `Its NICs will come back up once the new drivers load.`,
    );
    if (!ok) return;
    applyBusy = true;
    applyMessage = 'Applying modules (node will reboot)…';
    try {
      // Save the selection first so the effective set is persisted, then apply.
      await client.put(`/machines/${$page.params.id}/modules`, { modules: mods });
      effectiveModules = mods;
      modulesDirty = false;
      const res = await client.post(`/machines/${$page.params.id}/apply-modules`, {});
      const r = res as { image?: string };
      applyMessage = `Upgrade initiated with image ${r.image || ''}. The node is rebooting…`;
      success('Module upgrade started');
      // Re-prob the node after a delay so the new modules show up once it's back.
      setTimeout(() => void loadImageAndModules(true), 90_000);
    } catch (e: unknown) {
      applyMessage = '';
      notifyError(e instanceof Error ? e.message : 'Failed to apply modules');
    } finally {
      applyBusy = false;
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

  // ── cluster-default modules (for the "reset to cluster" affordance) ──
  async function loadClusterModules() {
    const cid = machine?.clusterId;
    if (!cid) {
      clusterModules = [];
      return;
    }
    clusterModulesBusy = true;
    try {
      const c = (await client.get(`/clusters/${cid}`)) as { factoryModules?: string[] };
      clusterModules = c.factoryModules || [];
    } catch {
      clusterModules = [];
    } finally {
      clusterModulesBusy = false;
    }
  }

  async function resetModules() {
    if (applyBusy) return;
    if (
      !confirm(
        'Reset this machine to the cluster default module set? ' +
          'This clears the node-level override. Then it will upgrade (reboot) to the cluster image.',
      )
    )
      return;
    applyBusy = true;
    applyMessage = 'Resetting to cluster modules (node will reboot)…';
    try {
      // Clear absolute override + any deltas via the delta endpoint (reset=true).
      await client.put(`/machines/${$page.params.id}/module-overrides`, {
        adds: null,
        removes: null,
        reset: true,
      });
      effectiveModules = [...clusterModules];
      editModules = new Set(effectiveModules);
      modulesDirty = false;
      const res = await client.post(`/machines/${$page.params.id}/apply-modules`, {});
      const r = res as { image?: string };
      applyMessage = `Upgrade initiated with image ${r.image || ''}. The node is rebooting…`;
      success('Reset to cluster modules started');
      setTimeout(() => void loadImageAndModules(true), 90_000);
    } catch (e: unknown) {
      applyMessage = '';
      notifyError(e instanceof Error ? e.message : 'Failed to reset modules');
    } finally {
      applyBusy = false;
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
      <div class="detail-title">
        {#if machine.clusterId}
          <a class="back-link" href="/clusters/{machine.clusterId}"
             title="Back to this cluster's machine list">← Back to machine list</a>
        {:else}
          <button class="back-link" type="button" onclick={() => history.back()}
                  title="Back to previous page">← Back</button>
        {/if}
        <h1>{hostnameLive || machine.hostname || machineLabel(machine)}</h1>
      </div>
      <div class="header-actions">
        <span class="status-badge">{machine.status}</span>
        <span class="type-badge">{roleLabel(machine.machineType)}</span>
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
        <div class="info-row">
          <span class="label">Management path</span>
          {#if machine.viaSiderolink}
            <span class="value">
              <span class="badge tunnel" title="TCS reaches this node through its Siderolink WireGuard tunnel (NAT/firewall safe), not the LAN address">via Siderolink tunnel</span>
              <span class="value mono">{machine.effectiveEndpoint || machine.siderolinkIp || '—'}</span>
            </span>
          {:else}
            <span class="value" title="TCS reaches this node directly at its LAN/inventory address">
              <span class="badge lan">direct</span>
              <span class="value mono">{machine.effectiveEndpoint || machine.address || '—'}</span>
            </span>
          {/if}
        </div>
        <div class="info-row"><span class="label">Created</span><span class="value">{machine.createdAt ? new Date(machine.createdAt).toLocaleString() : '—'}</span></div>
        <div class="form-row">
          <label>Hostname<input type="text" title="Node hostname as it appears in Talos" bind:value={editHostname} placeholder="cp-1" /></label>
        </div>
        <div class="form-row">
          <label>
            Role
            <select title="Node role: control-plane or worker" bind:value={editMachineType}>
              <option value="controlplane">control-plane</option>
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
        <h2>Image &amp; modules</h2>
        <div class="info-row">
          <span class="label">Running version</span>
          <span class="value mono">{versions?.version || machine.talosVersion || (extError ? '—' : (extBusy ? 'probing…' : 'unknown'))}</span>
        </div>
        {#if versions?.installed}
          <div class="info-row">
            <span class="label">Installed image</span>
            <span class="value mono">{versions.installed}</span>
          </div>
        {/if}
        {#if versions?.upgradable}
          <div class="info-row">
            <span class="label">Upgradable to</span>
            <span class="value mono">{versions.upgradable}</span>
          </div>
        {/if}
        <Button variant="ghost" size="sm" title="Re-probe the node's installed image and modules" onclick={() => loadImageAndModules(false)} disabled={extBusy}>Refresh</Button>
        {#if extError}
          <p class="muted-hint error-hint">{extError}</p>
        {/if}

        <h3 class="subheading">Installed modules</h3>
        {#if extensions.length === 0 && !extBusy && !extError}
          <p class="muted-hint">None installed.</p>
        {:else}
          <table class="modules-table">
            <thead>
              <tr><th>Module</th><th>Image</th><th>Version / hash</th></tr>
            </thead>
            <tbody>
              {#each extensions as ext (ext.id)}
                <tr>
                  <td class="mono">{ext.id}</td>
                  <td class="mono" title={ext.source}>{ext.source || '—'}</td>
                  <td class="mono" title={ext.hash}>{ext.hash ? (ext.hash.length > 16 ? ext.hash.slice(0, 16) + '…' : ext.hash) : '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}

        {#if effectiveModules.length > 0 && !extBusy}
          {@const missing = effectiveModules.filter((m) => !extensions.some((e) => e.id === shortModuleName(m)))}
          {#if missing.length > 0}
            <p class="muted-hint warn-hint">
              ⚠ Configured but not loaded yet: <span class="mono">{missing.map(shortModuleName).join(', ')}</span>.
              Use <strong>Apply modules</strong> to upgrade the node so they load.
            </p>
          {:else}
            <p class="muted-hint ok-hint">✓ All configured modules are loaded on this node.</p>
          {/if}
        {/if}

        <h3 class="subheading">Modules (Image Factory)</h3>
        <p class="muted-hint">
          Pick the system extensions to bake into this node's image. Applying upgrades the node to a
          factory image that bundles them (reboots the node, then its NICs come up with the new drivers).
        </p>
        {#if factoryError}
          <p class="muted-hint error-hint">{factoryError}</p>
        {:else if factoryBusy}
          <p class="muted-hint">Loading module catalog…</p>
        {:else}
          {#if factoryVersions.length > 0}
            <div class="form-row">
              <label>
                Talos version
                <select title="Talos version whose official modules to list" bind:value={selectedVersion} onchange={() => loadFactoryCatalog(selectedVersion)}>
                  {#each factoryVersions as v (v)}
                    <option value={v}>{v}</option>
                  {/each}
                </select>
              </label>
            </div>
          {/if}
          <div class="module-picker">
            {#each factoryExtensions as f (f.name)}
              <label class="module-option" title={f.description || f.ref || ''}>
                <input type="checkbox" checked={editModules.has(f.name)} onchange={() => toggleModule(f.name)} />
                <span class="mono">{shortModuleName(f.name)}</span>
                {#if f.author}<span class="muted-hint"> · {f.author}</span>{/if}
              </label>
            {/each}
            {#if factoryExtensions.length === 0}
              <p class="muted-hint">No modules returned for {selectedVersion || 'this version'}.</p>
            {/if}
          </div>
          <div class="module-selected" class:empty={editModules.size === 0}>
            {#if editModules.size > 0}
              <span class="muted-hint">Selected:</span>
              {#each [...editModules].sort() as m (m)}
                <span class="module-chip mono">{shortModuleName(m)}</span>
              {/each}
            {:else}
              <span class="muted-hint">None selected (no modules baked in).</span>
            {/if}
          </div>
          {#if clusterModules.length > 0 || clusterModulesBusy}
            <p class="muted-hint">
              {#if clusterModulesBusy}
                Loading cluster defaults…
              {:else}
                Cluster defaults:
                {#if clusterModules.length > 0}
                  {#each clusterModules as m (m)}
                    <span class="module-chip mono">{shortModuleName(m)}</span>
                  {/each}
                {:else}
                  <span>(none)</span>
                {/if}
              {/if}
            </p>
          {/if}
          <p class="muted-hint" style="margin-top:0.4rem">
            <strong>Node-level deltas</strong> — extra modules to add on top of the cluster
            defaults, or cluster defaults to remove from just this node. Effective set
            = cluster − removes + adds (an absolute "Apply modules" selection wins if set).
          </p>
          {#if factoryExtensions.length > 0 || effectiveModules.length > 0}
            <div class="module-picker" style="max-height:120px">
              {#each [...new Set([...clusterModules, ...effectiveModules, ...factoryExtensions.map((f) => f.name)])].sort() as m (m)}
                <span class="module-option">
                  <button
                    class="delta-btn"
                    class:active={adds.includes(m)}
                    title="Add {shortModuleName(m)} to this node only"
                    onclick={() => toggleAdd(m)}
                    disabled={overridesBusy}
                  >+</button>
                  <button
                    class="delta-btn"
                    class:active={removes.includes(m)}
                    title="Remove {shortModuleName(m)} from this node only"
                    onclick={() => toggleRemove(m)}
                    disabled={overridesBusy}
                  >−</button>
                  <span class="mono">{shortModuleName(m)}</span>
                  {#if isClusterDefault(m)}<span class="muted-hint"> · default</span>{/if}
                </span>
              {/each}
            </div>
            {#if adds.length > 0 || removes.length > 0}
              <p class="muted-hint">
                Adds: {adds.length ? adds.map(shortModuleName).join(', ') : '—'} ·
                Removes: {removes.length ? removes.map(shortModuleName).join(', ') : '—'}
              </p>
            {/if}
          {/if}
          <div class="module-actions">
            <Button variant="secondary" size="sm" title="Save the node-level add/remove deltas (no reboot)" onclick={saveOverrides} disabled={overridesBusy || (adds.length === 0 && removes.length === 0)}>Save deltas</Button>
            <Button variant="primary" size="sm" title="Upgrade the node to a factory image with these modules (reboots)" onclick={applyModules} disabled={applyBusy || editModules.size === 0}>Apply modules</Button>
            <Button
              variant="ghost"
              size="sm"
              title="Clear this node's override so it uses the cluster default module set, then upgrade (reboots)"
              onclick={resetModules}
              disabled={applyBusy}
            >Reset to cluster defaults</Button>
          </div>
          {#if applyMessage}
            <p class="muted-hint info-hint">{applyMessage}</p>
          {/if}
        {/if}
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
          <Button variant="secondary" size="sm" title="Open the out-of-band console (iLO HTML5 for HPE, SOL for Dell)" onclick={openConsole} disabled={actionBusy}>Console</Button>
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
        Edit the full Talos machine config for this node (network, mounts, and for the
        rare factory/custom install image case, <code>machine.install.image</code>). Save as
        desired copy, dry-run, then apply. Requires cluster talosconfig and a reachable
        machine address for live/apply.
        {#if hasDesired}<span class="badge">desired saved</span>{/if}
        {#if liveReachable}<span class="badge ok">node reachable</span>{:else}<span class="badge">live unknown</span>{/if}
      </p>

      <div class="helper-grid">
        <div class="info-section">
          <h3>Helpers (deep-merged into desired config)</h3>
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
  {/if}

  {#if consoleOpen}
    <div class="console-overlay" role="dialog" aria-modal="true"
         onkeydown={(e) => { if (e.key === 'Escape') closeConsole(); }}>
      <div class="console-bar">
        <span class="console-title">
          {#if consoleBusy}
            <Spinner size="sm" /> Opening…
          {:else}
            OOB console — {machine?.hostname || 'machine'}
          {/if}
          {#if consoleMode === 'ilo'}<span class="badge">iLO</span>{/if}
          {#if consoleMode === 'sol'}<span class="badge">SOL</span>{/if}
        </span>
        <span class="console-actions">
          {#if consoleMode === 'sol' && consoleIdracUrl}
            <a class="console-link" href={consoleIdracUrl} target="_blank" rel="noopener"
               title="Open the iDRAC in a new tab (goes straight to the video console after login)">iDRAC video console ↗</a>
            <Button variant="secondary" size="sm" title="Show the iDRAC username/password" onclick={showIdracCreds}>Show iDRAC login</Button>
          {/if}
          <Button variant="secondary" size="sm" title="Close the console (Esc)" onclick={closeConsole}>Close</Button>
        </span>
      </div>
      <div class="console-body">
        {#if consoleError}
          <div class="console-error">{consoleError}</div>
        {/if}
        {#if consoleMode === 'sol' && consoleIdracUrl}
          <div class="console-hint">
            The iDRAC <b>video console</b> opened in a new tab (log in there —
            "Show iDRAC login" reveals the credentials). Below is the
            <b>SOL serial terminal</b> as a fallback.
          </div>
        {/if}
        {#if consoleMode === 'ilo'}
          <iframe class="ilo-frame" src={consoleEmbed} title="iLO remote console"></iframe>
        {:else if consoleMode === 'sol'}
          <div class="sol-term" use:solContainer></div>
        {:else}
          <div class="console-empty">Console unavailable.</div>
        {/if}
      </div>

      {#if idracCredsOpen}
        <div class="idrac-creds-backdrop" role="dialog" aria-modal="true" onclick={closeIdracCreds}>
          <div class="idrac-creds" onclick={(e) => e.stopPropagation()}>
            <div class="idrac-creds-head">
              <span>iDRAC login — {machine?.hostname || 'machine'}</span>
              <Button variant="ghost" size="sm" title="Close (Esc)" onclick={closeIdracCreds}>✕</Button>
            </div>
            <div class="idrac-creds-body">
              {#if idracCredsLoading}
                <div class="idrac-creds-loading"><Spinner size="sm" /> Loading credentials…</div>
              {:else if idracCreds}
                <div class="cred-row">
                  <label>Username</label>
                  <div class="cred-val">
                    <input readonly value={idracCreds.username} />
                    <Button variant="ghost" size="sm" title="Copy username" onclick={() => copyCred('username')}>copy</Button>
                  </div>
                </div>
                <div class="cred-row">
                  <label>Password</label>
                  <div class="cred-val">
                    <input type={idracCredsRevealed ? 'text' : 'password'} value={idracCreds.password} readonly />
                    <Button variant="ghost" size="sm" title={idracCredsRevealed ? 'Hide' : 'Show'}
                            onclick={() => (idracCredsRevealed = !idracCredsRevealed)}>{idracCredsRevealed ? 'hide' : 'show'}</Button>
                    <Button variant="ghost" size="sm" title="Copy password" onclick={() => copyCred('password')}>copy</Button>
                  </div>
                </div>
                <a class="idrac-creds-link" href={idracCreds.idracUrl} target="_blank" rel="noopener">
                  Open iDRAC console ↗ <span class="muted">(login.html?console)</span>
                </a>
                <p class="muted small">
                  Enter these in the iDRAC login tab that opened. The password is
                  shown on request only and is not stored in your browser.
                </p>
              {:else}
                <div class="idrac-creds-error">{consoleError || 'Unable to load credentials'}</div>
              {/if}
            </div>
          </div>
        </div>
      {/if}
    </div>
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
  .detail-title { display: flex; flex-direction: column; gap: 0.25rem; }
  .back-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font-size: 0.85rem;
    color: var(--tcs-text-muted, #9aa4b2);
    text-decoration: none;
  }
  .back-link:hover { color: var(--tcs-text, #e6e9ef); text-decoration: underline; }
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
  .value { display: inline-flex; align-items: center; gap: 0.5rem; }
  .badge {
    font-size: 0.7rem;
    font-weight: 600;
    padding: 0.1rem 0.45rem;
    border-radius: 10px;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }
  .badge.tunnel { background: rgba(34, 197, 94, 0.15); color: var(--tcs-secondary); border: 1px solid rgba(34, 197, 94, 0.4); }
  .badge.lan { background: var(--tcs-surface); color: var(--tcs-text-muted); border: 1px solid var(--tcs-border); }
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
  .muted-hint code {
    font-family: var(--tcs-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    background: rgba(255, 255, 255, 0.06);
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }
  .error-hint { color: var(--tcs-error, #ef4444); }
  .subheading {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--tcs-text-muted);
    margin: 1rem 0 0.5rem;
  }
  .modules-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }
  .modules-table th, .modules-table td {
    text-align: left;
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .modules-table th { color: var(--tcs-text-muted); font-weight: 600; }
  .warn-hint { color: var(--tcs-warning, #f59e0b); }
  .ok-hint { color: var(--tcs-success, #22c55e); }
  .info-hint { color: var(--tcs-text, #e5e7eb); }
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
    background: var(--tcs-surface-2, rgba(255,255,255,0.02));
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
  .module-option .muted-hint {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .module-selected {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin: 0.4rem 0;
    min-height: 1.4rem;
  }
  .module-chip {
    background: var(--tcs-accent-soft, rgba(59,130,246,0.15));
    border: 1px solid var(--tcs-accent, #3b82f6);
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.75rem;
  }
  .module-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-top: 0.4rem; }
  .delta-btn {
    all: unset;
    width: 1.4rem; height: 1.4rem;
    display: inline-flex; align-items: center; justify-content: center;
    border: 1px solid var(--tcs-border);
    border-radius: 999px;
    font-size: 0.9rem; line-height: 1;
    cursor: pointer;
    color: var(--tcs-text-muted);
    margin-right: 0.15rem;
  }
  .delta-btn:hover { border-color: var(--tcs-primary); color: var(--tcs-primary); }
  .delta-btn.active { background: var(--tcs-primary); border-color: var(--tcs-primary); color: #fff; }
  .delta-btn[disabled] { opacity: 0.4; cursor: default; }
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

  /* ---- OOB console overlay ---- */
  .console-overlay {
    position: fixed; inset: 0; z-index: 1000;
    background: #05070b;
    display: flex; flex-direction: column;
  }
  .console-bar {
    display: flex; align-items: center; justify-content: space-between;
    padding: 0.4rem 0.9rem;
    background: #0b0e14;
    border-bottom: 1px solid var(--tcs-border);
    color: #e6e8ee;
  }
  .console-title { font-weight: 600; font-size: 0.95rem; display: flex; gap: 0.5rem; align-items: center; }
  .console-actions { display: flex; gap: 0.6rem; align-items: center; }
  .console-link {
    color: #60a5fa; text-decoration: none; font-size: 0.85rem;
    padding: 0.35rem 0.6rem; border: 1px solid var(--tcs-border); border-radius: 6px;
  }
  .console-link:hover { background: rgba(96,165,250,0.1); }
  .console-body { flex: 1; position: relative; overflow: hidden; display: flex; flex-direction: column; }
  .console-hint {
    flex: 0 0 auto; padding: 0.35rem 0.75rem; font-size: 0.78rem; line-height: 1.3;
    color: #bae6fd; background: rgba(14,165,233,0.12); border-bottom: 1px solid rgba(14,165,233,0.25);
  }
  .idrac-creds-backdrop {
    position: absolute; inset: 0; z-index: 5; background: rgba(0,0,0,0.55);
    display: flex; align-items: center; justify-content: center; padding: 1rem;
  }
  .idrac-creds {
    background: #0f172a; border: 1px solid #334155; border-radius: 10px;
    width: min(460px, 100%); box-shadow: 0 12px 40px rgba(0,0,0,0.5);
  }
  .idrac-creds-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 0.6rem 0.8rem; border-bottom: 1px solid #1e293b; font-weight: 600; font-size: 0.9rem;
  }
  .idrac-creds-body { padding: 0.9rem 0.8rem; display: flex; flex-direction: column; gap: 0.7rem; }
  .idrac-creds-loading { display: flex; align-items: center; gap: 0.5rem; color: #94a3b8; font-size: 0.85rem; }
  .idrac-creds-error { color: #fca5a5; font-size: 0.85rem; }
  .cred-row { display: flex; flex-direction: column; gap: 0.25rem; }
  .cred-row label { font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.04em; color: #94a3b8; }
  .cred-val { display: flex; align-items: center; gap: 0.35rem; }
  .cred-val input {
    flex: 1; min-width: 0; background: #020617; border: 1px solid #334155; border-radius: 6px;
    color: #e2e8f0; padding: 0.35rem 0.5rem; font-family: ui-monospace, monospace; font-size: 0.85rem;
  }
  .idrac-creds-link { color: #60a5fa; font-size: 0.85rem; text-decoration: none; }
  .idrac-creds-link:hover { text-decoration: underline; }
  .idrac-creds .muted { color: #94a3b8; }
  .idrac-creds .small { font-size: 0.75rem; }
  .console-error {
    position: absolute; top: 0.5rem; left: 50%; transform: translateX(-50%);
    z-index: 2; background: rgba(248,113,113,0.15); color: #fecaca;
    border: 1px solid rgba(248,113,113,0.4); border-radius: 6px;
    padding: 0.3rem 0.8rem; font-size: 0.8rem; max-width: 80%;
  }
  .ilo-frame { width: 100%; flex: 1 1 auto; min-height: 0; border: 0; background: #000; display: block; }
  .sol-term { width: 100%; flex: 1 1 auto; min-height: 0; padding: 0.25rem; background: #0b0e14; }
  .console-empty { color: var(--tcs-text-muted); padding: 2rem; text-align: center; }
</style>
