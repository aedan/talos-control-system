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

  onMount(async () => {
    try {
      machine = (await client.get(`/machines/${$page.params.id}`)) as Machine;
      editAddress = machine.address || '';
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine';
    } finally {
      loading = false;
    }
  });

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
      <h1>{hostnameLive || machineLabel(machine)}</h1>
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
        <h2>Identity</h2>
        <div class="info-row"><span class="label">System UUID</span><span class="value mono">{machine.systemUuid}</span></div>
        <div class="info-row"><span class="label">Cluster</span><span class="value mono">{machine.clusterId || '—'}</span></div>
        <div class="info-row"><span class="label">Talos</span><span class="value">{machine.talosVersion || '—'}</span></div>
        <div class="info-row"><span class="label">Secure boot</span><span class="value">{machine.secureBoot ? 'Yes' : 'No'}</span></div>
        <div class="info-row"><span class="label">Created</span><span class="value">{machine.createdAt ? new Date(machine.createdAt).toLocaleString() : '—'}</span></div>
      </div>

      <div class="info-section">
        <h2>Talos connectivity</h2>
        <div class="form-row">
          <label>
            Address
            <input type="text" bind:value={editAddress} placeholder="10.0.0.2 or host:50000" />
          </label>
          <Button variant="secondary" size="sm" onclick={saveAddress} disabled={actionBusy}>Save</Button>
        </div>
        <div class="form-row">
          <label>
            Upgrade image
            <input type="text" bind:value={upgradeImage} placeholder="ghcr.io/siderolabs/installer:v1.8.0" />
          </label>
          <Button variant="secondary" size="sm" onclick={upgrade} disabled={actionBusy}>Upgrade</Button>
        </div>
      </div>
    </div>

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
