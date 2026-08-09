<script lang="ts">
  import { page } from '$app/stores';
  import { client } from '$lib/api/client';
  import { onMount } from 'svelte';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';

  let cluster = $state(null as any);
  let loading = $state(true);
  let error = $state('');
  let busy = $state(false);
  let talosconfigText = $state('');

  onMount(async () => {
    try {
      cluster = await client.get(`/clusters/${$page.params.id}`);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load cluster';
    } finally {
      loading = false;
    }
  });

  async function refresh() {
    busy = true;
    try {
      const res = (await client.post(`/clusters/${$page.params.id}/refresh`, {})) as {
        machines: number;
      };
      cluster = await client.get(`/clusters/${$page.params.id}`);
      success(`Refreshed ${res.machines} machine(s) from kubeconfig`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Refresh failed (need stored kubeconfig)');
    } finally {
      busy = false;
    }
  }

  async function testTalos() {
    busy = true;
    try {
      const res = (await client.post(`/clusters/${$page.params.id}/talos/test`, {})) as {
        results: Array<{ ok: boolean; address?: string; talosVersion?: string; error?: string }>;
      };
      const ok = res.results?.filter((r) => r.ok).length ?? 0;
      const fail = (res.results?.length ?? 0) - ok;
      success(`Talos test: ${ok} ok, ${fail} failed`);
      console.info('talos test', res.results);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Talos connectivity test failed');
    } finally {
      busy = false;
    }
  }

  async function saveTalosconfig() {
    if (!talosconfigText.trim()) return;
    busy = true;
    try {
      await client.put(`/clusters/${$page.params.id}/talosconfig`, {
        talosconfig: talosconfigText,
      });
      cluster = await client.get(`/clusters/${$page.params.id}`);
      talosconfigText = '';
      success('Talosconfig saved (encrypted at rest)');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to save talosconfig');
    } finally {
      busy = false;
    }
  }
</script>

<div class="cluster-detail">
{#if loading}
    <p>Loading...</p>
{:else if error}
    <div class="error-banner">{error}</div>
{:else if cluster}
    <div class="header-row">
      <h1>{cluster.name}</h1>
      <div class="actions">
        <Button variant="secondary" size="sm" onclick={refresh} disabled={busy}>Refresh from K8s</Button>
        <Button variant="secondary" size="sm" onclick={testTalos} disabled={busy}>Test Talos</Button>
      </div>
    </div>

    <div class="info-grid">
      <div class="info-item">
        <span class="info-label">Kubernetes</span>
        <span class="info-value">{cluster.controlPlaneVersion || cluster.control_plane_version}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Talos</span>
        <span class="info-value">{cluster.talosVersion || cluster.talos_version}</span>
      </div>
      <div class="info-item">
        <span class="info-label">Status</span>
        <span class="info-value">{cluster.status}</span>
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

    <details class="talos-attach">
      <summary>Attach / replace talosconfig</summary>
      <textarea bind:value={talosconfigText} rows="8" placeholder="Paste ~/.talos/config YAML"></textarea>
      <Button variant="primary" size="sm" onclick={saveTalosconfig} disabled={busy || !talosconfigText.trim()}>
        Save talosconfig
      </Button>
    </details>

    <nav class="tabs">
      <a href="/clusters/{$page.params.id}/nodes">Nodes</a>
      <a href="/clusters/{$page.params.id}/machines">Machines</a>
      <a href="/clusters/{$page.params.id}/config">Config</a>
      <a href="/clusters/{$page.params.id}/backups">Backups</a>
    </nav>
  {/if}
</div>

<style>
  .cluster-detail h1 { margin: 0; }
  .header-row { display: flex; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1.5rem; flex-wrap: wrap; }
  .actions { display: flex; gap: 0.5rem; }
  .info-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }
  .info-item { display: flex; flex-direction: column; gap: 0.25rem; }
  .info-label { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .info-value { font-size: 1.05rem; font-weight: 500; }

  .talos-attach { margin-bottom: 1.5rem; background: var(--tcs-surface); border: 1px solid var(--tcs-border); border-radius: 8px; padding: 0.75rem 1rem; }
  .talos-attach textarea { width: 100%; margin: 0.75rem 0; font-family: ui-monospace, monospace; font-size: 0.8rem; }

  .tabs { display: flex; gap: 0; border-bottom: 1px solid var(--tcs-border); margin-bottom: 1.5rem; }
  .tabs a {
    padding: 0.75rem 1rem;
    color: var(--tcs-text-muted);
    border-bottom: 2px solid transparent;
    transition: all 0.15s;
  }
  .tabs a:hover { color: var(--tcs-text); text-decoration: none; }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem 1rem;
    color: var(--tcs-error, #ef4444);
    font-size: 0.875rem;
    margin-bottom: 1.5rem;
  }
</style>
