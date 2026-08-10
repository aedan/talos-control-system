<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface UpgradeJob {
    id: string;
    scope: string;
    image: string;
    status: string;
    maxUnavailable?: number;
    max_unavailable?: number;
    controlPlaneLast?: boolean;
    cancelRequested?: boolean;
    createdAt?: string;
    created_at?: string;
    error?: string | null;
  }

  interface ClusterRow {
    id: string;
    name: string;
  }

  let jobs = $state<UpgradeJob[]>([]);
  let clusters = $state<ClusterRow[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let selectedDetail = $state<any>(null);

  let fleetImage = $state('ghcr.io/siderolabs/installer:v1.9.0');
  let fleetClusterIds = $state<string[]>([]);
  let maxUnavailable = $state(1);
  let controlPlaneLast = $state(true);

  async function load() {
    loading = true;
    try {
      const [j, c] = await Promise.all([
        client.get('/upgrade-jobs') as Promise<UpgradeJob[]>,
        client.get('/clusters') as Promise<ClusterRow[]>,
      ]);
      jobs = j || [];
      clusters = c || [];
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load upgrades');
    } finally {
      loading = false;
    }
  }

  onMount(load);

  function toggleCluster(id: string) {
    if (fleetClusterIds.includes(id)) {
      fleetClusterIds = fleetClusterIds.filter((x) => x !== id);
    } else {
      fleetClusterIds = [...fleetClusterIds, id];
    }
  }

  async function startFleet() {
    if (!fleetImage.trim() || fleetClusterIds.length === 0) {
      notifyError('Select at least one cluster and set an image');
      return;
    }
    if (!confirm(`Start fleet upgrade to ${fleetImage} for ${fleetClusterIds.length} cluster(s)?`)) {
      return;
    }
    busy = true;
    try {
      await client.post('/fleets/upgrades', {
        clusterIds: fleetClusterIds,
        image: fleetImage.trim(),
        maxUnavailable,
        controlPlaneLast,
      });
      success('Fleet upgrade job queued');
      await load();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to start fleet upgrade');
    } finally {
      busy = false;
    }
  }

  async function openJob(id: string) {
    try {
      selectedDetail = await client.get(`/upgrade-jobs/${id}`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load job');
    }
  }

  async function cancelJob(id: string) {
    if (!confirm('Request cancel for this upgrade job?')) return;
    busy = true;
    try {
      await client.post(`/upgrade-jobs/${id}/cancel`, {});
      success('Cancel requested');
      await load();
      if (selectedDetail?.job?.id === id) {
        selectedDetail = await client.get(`/upgrade-jobs/${id}`);
      }
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Cancel failed');
    } finally {
      busy = false;
    }
  }
</script>

<div class="upgrades-page">
  <div class="header-row">
    <div>
      <h1>Rolling upgrades</h1>
      <p class="desc">
        Cluster and fleet Talos upgrades with max-unavailable and optional control-plane-last ordering.
      </p>
    </div>
    <Button variant="secondary" size="sm" onclick={load} disabled={loading || busy}>Refresh</Button>
  </div>

  <section class="card">
    <h2>Fleet upgrade</h2>
    <div class="form-row">
      <label>
        Installer image
        <input type="text" bind:value={fleetImage} placeholder="ghcr.io/siderolabs/installer:v1.x" />
      </label>
      <label class="num">
        Max unavailable
        <input type="number" min="1" max="20" bind:value={maxUnavailable} />
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={controlPlaneLast} />
        Workers first (control plane last)
      </label>
    </div>
    <div class="cluster-pick">
      {#each clusters as c}
        <label class="chip">
          <input
            type="checkbox"
            checked={fleetClusterIds.includes(c.id)}
            onchange={() => toggleCluster(c.id)}
          />
          {c.name}
        </label>
      {/each}
      {#if clusters.length === 0}
        <p class="muted">No clusters imported yet.</p>
      {/if}
    </div>
    <Button variant="primary" onclick={startFleet} disabled={busy}>Start fleet upgrade</Button>
  </section>

  <section class="card">
    <h2>Jobs</h2>
    {#if loading}
      <Spinner />
    {:else if jobs.length === 0}
      <p class="muted">No upgrade jobs yet. Start one from a cluster page or fleet form above.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Scope</th>
            <th>Image</th>
            <th>Status</th>
            <th>Created</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each jobs as j}
            <tr>
              <td>{j.scope}</td>
              <td class="mono">{j.image}</td>
              <td><span class="status status-{j.status}">{j.status}</span></td>
              <td>{new Date(j.createdAt || j.created_at || '').toLocaleString()}</td>
              <td class="row-actions">
                <Button variant="ghost" size="sm" onclick={() => openJob(j.id)}>Details</Button>
                {#if j.status === 'pending' || j.status === 'running'}
                  <Button variant="ghost" size="sm" onclick={() => cancelJob(j.id)} disabled={busy}
                    >Cancel</Button
                  >
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  {#if selectedDetail}
    <section class="card">
      <h2>Job detail</h2>
      <pre class="detail">{JSON.stringify(selectedDetail, null, 2)}</pre>
    </section>
  {/if}
</div>

<style>
  .upgrades-page h1 {
    margin: 0 0 0.35rem;
  }
  .desc {
    color: var(--tcs-text-muted);
    margin: 0;
    font-size: 0.9rem;
  }
  .header-row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.5rem;
  }
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1.25rem;
    margin-bottom: 1.25rem;
  }
  .card h2 {
    margin: 0 0 1rem;
    font-size: 1.05rem;
  }
  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 1rem;
    align-items: flex-end;
  }
  .form-row label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: var(--tcs-text-muted);
    flex: 1;
    min-width: 200px;
  }
  .form-row label.num {
    flex: 0 0 120px;
    min-width: 100px;
  }
  .form-row label.check {
    flex-direction: row;
    align-items: center;
    min-width: auto;
    color: var(--tcs-text);
  }
  input[type='text'],
  input[type='number'] {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.7rem;
    color: var(--tcs-text);
  }
  .cluster-pick {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.65rem;
    border: 1px solid var(--tcs-border);
    border-radius: 999px;
    font-size: 0.85rem;
  }
  .muted {
    color: var(--tcs-text-muted);
    font-size: 0.9rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.55rem 0.4rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }
  .status {
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.03em;
  }
  .status-completed {
    color: #4ade80;
  }
  .status-failed {
    color: #f87171;
  }
  .status-running {
    color: #60a5fa;
  }
  .row-actions {
    display: flex;
    gap: 0.25rem;
  }
  .detail {
    overflow: auto;
    max-height: 420px;
    font-size: 0.8rem;
    background: var(--tcs-background);
    padding: 0.75rem;
    border-radius: 6px;
  }
</style>
