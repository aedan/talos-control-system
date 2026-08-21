<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  let loading = $state(true);
  let saving = $state(false);
  let status = $state<any>(null);
  let profiles = $state<any[]>([]);
  let leases = $state<any[]>([]);
  let jobs = $state<any[]>([]);
  let newName = $state('metal-default');
  let newVersion = $state('v1.13.7');
  let syncing = $state<string | null>(null);

  // editable form
  let enabled = $state(false);
  let dhcpEnabled = $state(false);
  let dhcpIface = $state('');
  let dhcpBindIp = $state('');
  let dhcpSubnet = $state('10.88.0.0/24');
  let dhcpStart = $state('10.88.0.100');
  let dhcpEnd = $state('10.88.0.200');
  let dhcpGw = $state('10.88.0.1');
  let dhcpDns = $state('10.88.0.1');
  let dhcpAllowUnknown = $state(false);
  let pxeEnabled = $state(false);
  let pxePort = $state(6969);
  let pxeAssetDir = $state('/var/lib/tcs/pxe');
  let pxeTalos = $state('v1.13.7');

  function fillForm(s: any) {
    status = s;
    enabled = !!s.enabled;
    dhcpEnabled = !!s.dhcp?.enabled;
    dhcpIface = s.dhcp?.interface || '';
    dhcpBindIp = s.dhcp?.bindIp || '';
    dhcpSubnet = s.dhcp?.subnet || '10.88.0.0/24';
    dhcpStart = s.dhcp?.rangeStart || '10.88.0.100';
    dhcpEnd = s.dhcp?.rangeEnd || '10.88.0.200';
    dhcpGw = s.dhcp?.gateway || '10.88.0.1';
    dhcpDns = (s.dhcp?.dns || []).join(', ') || '10.88.0.1';
    dhcpAllowUnknown = !!s.dhcp?.allowUnknown;
    pxeEnabled = !!s.pxe?.enabled;
    pxePort = s.pxe?.httpPort || 6969;
    pxeAssetDir = s.pxe?.assetDir || '/var/lib/tcs/pxe';
    pxeTalos = s.pxe?.defaultTalosVersion || 'v1.13.7';
  }

  async function refresh() {
    loading = true;
    try {
      const s = await client.get('/settings/metal/config');
      fillForm(s);
      const p = (await client.get('/pxe/profiles')) as { profiles?: any[] };
      profiles = p.profiles || [];
      const l = (await client.get('/metal/dhcp/leases')) as { leases?: any[] };
      leases = l.leases || [];
      const j = (await client.get('/provision-jobs')) as { jobs?: any[] };
      jobs = j.jobs || [];
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load metal status');
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  async function applyConfig() {
    if (dhcpEnabled && !dhcpIface.trim() && !dhcpBindIp.trim()) {
      notifyError('DHCP requires interface or bind IP');
      return;
    }
    saving = true;
    try {
      const res = (await client.put('/settings/metal/config', {
        enabled,
        dhcp: {
          enabled: dhcpEnabled,
          interface: dhcpIface.trim(),
          bindIp: dhcpBindIp.trim(),
          subnet: dhcpSubnet.trim(),
          rangeStart: dhcpStart.trim(),
          rangeEnd: dhcpEnd.trim(),
          gateway: dhcpGw.trim(),
          dns: dhcpDns
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean),
          allowUnknown: dhcpAllowUnknown,
        },
        pxe: {
          enabled: pxeEnabled,
          httpPort: Number(pxePort) || 6969,
          assetDir: pxeAssetDir.trim(),
          defaultTalosVersion: pxeTalos.trim(),
        },
      })) as any;
      success(
        res.restartRequired
          ? 'Saved (restart still required)'
          : 'Metal config applied live (no process restart)'
      );
      await refresh();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Apply failed');
    } finally {
      saving = false;
    }
  }

  async function createProfile() {
    try {
      await client.post('/pxe/profiles', {
        name: newName,
        talosVersion: newVersion,
        arch: 'amd64',
      });
      success('PXE profile created');
      await refresh();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Create profile failed');
    }
  }

  async function syncProfile(id: string) {
    syncing = id;
    try {
      await client.post(`/pxe/profiles/${id}/sync`, {});
      success('Assets downloaded');
      await refresh();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Sync failed');
    } finally {
      syncing = null;
    }
  }
</script>

<div class="metal-settings">
  <h1>Metal / PXE</h1>
  <p class="hint">
    Configure provisioning DHCP and PXE here. Changes write
    <code>/var/lib/tcs/metal.toml</code> and rebind listeners <strong>without</strong>
    restarting the TCS process. Use a dedicated provision VLAN.
  </p>

  {#if loading}
    <Spinner />
  {:else}
    <section class="card">
      <h2>Live configuration</h2>
      {#if status?.liveReload}
        <p class="ok">Live apply available</p>
      {/if}
      <div class="form-grid">
        <label class="check"><input type="checkbox" title="Enable the metal provisioning master" bind:checked={enabled} /> Metal master enable</label>
        <label class="check"><input type="checkbox" title="Enable the DHCP server for PXE boot" bind:checked={dhcpEnabled} /> DHCP enabled</label>
        <label class="check"><input type="checkbox" title="Enable the PXE HTTP asset server" bind:checked={pxeEnabled} /> PXE HTTP enabled</label>
        <label>DHCP interface<input title="Network interface the DHCP server binds to" bind:value={dhcpIface} placeholder="eth1" /></label>
        <label>DHCP bind IP<input title="Specific IP for the DHCP server (optional)" bind:value={dhcpBindIp} placeholder="optional" /></label>
        <label>Subnet CIDR<input title="Provision VLAN subnet in CIDR notation" bind:value={dhcpSubnet} /></label>
        <label>Range start<input title="First IP in the DHCP lease range" bind:value={dhcpStart} /></label>
        <label>Range end<input title="Last IP in the DHCP lease range" bind:value={dhcpEnd} /></label>
        <label>Gateway<input title="Gateway handed out by DHCP" bind:value={dhcpGw} /></label>
        <label>DNS (comma-separated)<input title="DNS servers handed out by DHCP" bind:value={dhcpDns} /></label>
        <label class="check"
          ><input type="checkbox" title="Allow DHCP leases for MACs not in inventory" bind:checked={dhcpAllowUnknown} /> Allow unknown MACs</label
        >
        <label>PXE HTTP port<input type="number" title="HTTP port serving PXE assets" bind:value={pxePort} /></label>
        <label>Asset dir<input title="Directory storing downloaded PXE assets" bind:value={pxeAssetDir} /></label>
        <label>Default Talos version<input title="Default Talos version for PXE profiles" bind:value={pxeTalos} /></label>
      </div>
      <div class="row">
        <Button variant="primary" title="Save and apply the metal/DHCP/PXE config live (no process restart)" onclick={applyConfig} disabled={saving}>
          {saving ? 'Applying…' : 'Apply (no service restart)'}
        </Button>
        <Button variant="ghost" title="Reload the current metal configuration and status" onclick={refresh}>Refresh</Button>
      </div>
    </section>

    <section class="card">
      <h2>PXE profiles</h2>
      <div class="row">
        <input title="Name for the new PXE profile" bind:value={newName} placeholder="name" />
        <input title="Talos version for the new PXE profile" bind:value={newVersion} placeholder="v1.13.7" />
        <Button variant="secondary" size="sm" title="Create a new PXE profile" onclick={createProfile}>Add profile</Button>
      </div>
      <table class="data-table">
        <thead>
          <tr><th>Name</th><th>Version</th><th>Arch</th><th>Assets</th><th></th></tr>
        </thead>
        <tbody>
          {#each profiles as p (p.id)}
            <tr>
              <td>{p.name}</td>
              <td class="mono">{p.talosVersion}</td>
              <td>{p.arch}</td>
              <td>{p.assetsReady ? 'ready' : 'missing'}</td>
              <td>
                <Button
                  variant="secondary"
                  size="sm"
                  title="Download the Talos PXE assets for this profile"
                  disabled={syncing === p.id}
                  onclick={() => syncProfile(p.id)}
                >
                  {syncing === p.id ? 'Syncing…' : 'Sync assets'}
                </Button>
              </td>
            </tr>
          {:else}
            <tr><td colspan="5">No profiles yet</td></tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section class="card">
      <h2>DHCP leases</h2>
      <table class="data-table">
        <thead>
          <tr><th>MAC</th><th>IP</th><th>Hostname</th><th>Expires</th></tr>
        </thead>
        <tbody>
          {#each leases as l (l.mac)}
            <tr>
              <td class="mono">{l.mac}</td>
              <td class="mono">{l.ip}</td>
              <td>{l.hostname || '—'}</td>
              <td>{l.expiresAt ? new Date(l.expiresAt).toLocaleString() : '—'}</td>
            </tr>
          {:else}
            <tr><td colspan="4">No leases</td></tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section class="card">
      <h2>Provision jobs</h2>
      <table class="data-table">
        <thead>
          <tr><th>ID</th><th>Kind</th><th>Status</th><th>Updated</th></tr>
        </thead>
        <tbody>
          {#each jobs as j (j.id)}
            <tr>
              <td class="mono">{j.id?.slice?.(0, 8) || j.id}</td>
              <td>{j.kind}</td>
              <td>{j.status}</td>
              <td>{j.updatedAt ? new Date(j.updatedAt).toLocaleString() : '—'}</td>
            </tr>
          {:else}
            <tr><td colspan="4">No jobs</td></tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
</div>

<style>
  .hint { color: var(--tcs-text-muted); max-width: 48rem; }
  .ok { color: #4ade80; font-size: 0.85rem; }
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin: 1rem 0;
  }
  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 0.75rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8rem;
  }
  label.check { flex-direction: row; align-items: center; gap: 0.5rem; }
  input {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
  }
  .row { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-top: 0.75rem; align-items: center; }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.4rem 0.5rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  code { font-size: 0.85em; }
</style>
