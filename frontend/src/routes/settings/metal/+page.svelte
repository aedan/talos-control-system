<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  let loading = $state(true);
  let status = $state<any>(null);
  let profiles = $state<any[]>([]);
  let leases = $state<any[]>([]);
  let jobs = $state<any[]>([]);
  let newName = $state('metal-default');
  let newVersion = $state('v1.13.7');
  let syncing = $state<string | null>(null);

  async function refresh() {
    loading = true;
    try {
      status = await client.get('/metal/status');
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
    Full DHCP + PXE is configured in <code>/etc/tcs/config.toml</code> under
    <code>[metal]</code>. Use a dedicated provisioning VLAN — DHCP is disabled by default.
  </p>

  {#if loading}
    <Spinner />
  {:else if status}
    <section class="card">
      <h2>Runtime status</h2>
      <div class="grid">
        <div><span class="label">Metal master</span> {status.enabled ? 'on' : 'off'}</div>
        <div><span class="label">DHCP</span> {status.dhcp?.enabled ? 'enabled' : 'disabled'} · {status.dhcp?.interface || '—'}</div>
        <div><span class="label">DHCP range</span> {status.dhcp?.rangeStart} – {status.dhcp?.rangeEnd}</div>
        <div><span class="label">PXE HTTP</span> {status.pxe?.enabled ? `:${status.pxe.httpPort}` : 'disabled'}</div>
        <div><span class="label">Default Talos</span> {status.pxe?.defaultTalosVersion}</div>
        <div><span class="label">Asset dir</span> <code>{status.pxe?.assetDir}</code></div>
      </div>
    </section>

    <section class="card">
      <h2>PXE profiles</h2>
      <div class="row">
        <input bind:value={newName} placeholder="name" />
        <input bind:value={newVersion} placeholder="v1.13.7" />
        <Button variant="secondary" size="sm" onclick={createProfile}>Add profile</Button>
        <Button variant="ghost" size="sm" onclick={refresh}>Refresh</Button>
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
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin: 1rem 0;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.75rem;
    font-size: 0.9rem;
  }
  .label { color: var(--tcs-text-muted); display: block; font-size: 0.75rem; }
  .row { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-bottom: 0.75rem; }
  input {
    padding: 0.4rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    background: var(--tcs-background);
    color: var(--tcs-text);
  }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.4rem 0.5rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  code { font-size: 0.85em; }
</style>
