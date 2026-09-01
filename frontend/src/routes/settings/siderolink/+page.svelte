<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  let peers = $state<any[]>([]);
  let tokens = $state<any[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let label = $state('');
  let expiresHours = $state(168);
  let lastToken = $state('');

  // Per-cluster tokens
  let clusters = $state<Array<{ id: string; name: string }>>([]);
  let selectedCluster = $state('');
  let clusterToken = $state('');
  let clusterEndpoint = $state('');
  let clusterTokenLoading = $state(false);
  let clusterTokenBusy = $state(false);
  let copied = $state(false);

  async function load() {
    loading = true;
    try {
      const [p, t, c] = await Promise.all([
        client.get('/siderolink/peers') as Promise<any[]>,
        client.get('/siderolink/tokens') as Promise<any[]>,
        client.get('/clusters') as Promise<any[]>,
      ]);
      peers = p || [];
      tokens = t || [];
      clusters = c || [];
      if (!selectedCluster && clusters.length > 0) selectedCluster = clusters[0].id;
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load Siderolink data');
    } finally {
      loading = false;
    }
  }

  onMount(load);

  $effect(() => {
    if (selectedCluster) loadClusterToken();
  });

  async function loadClusterToken() {
    clusterTokenLoading = true;
    try {
      const res = (await client.get(
        `/siderolink/cluster-token?cluster_id=${selectedCluster}`,
      )) as { token: string | null; endpoint: string };
      clusterToken = res.token || '';
      clusterEndpoint = res.endpoint || '';
    } catch {
      clusterToken = '';
    } finally {
      clusterTokenLoading = false;
    }
  }

  async function rotateClusterToken() {
    clusterTokenBusy = true;
    try {
      const res = (await client.post('/siderolink/cluster-token/rotate', {
        cluster_id: selectedCluster,
      })) as { token: string; endpoint: string };
      clusterToken = res.token || '';
      clusterEndpoint = res.endpoint || '';
      success('Cluster token rotated');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to rotate token');
    } finally {
      clusterTokenBusy = false;
    }
  }

  async function revokeClusterToken() {
    if (!confirm('Revoke this cluster’s Siderolink token? Machines using it will not be able to (re)register.')) return;
    clusterTokenBusy = true;
    try {
      await client.post('/siderolink/cluster-token/revoke', { cluster_id: selectedCluster });
      clusterToken = '';
      success('Cluster token revoked');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to revoke token');
    } finally {
      clusterTokenBusy = false;
    }
  }

  function clusterSnippet() {
    if (!clusterToken || !clusterEndpoint) return '';
    return `  siderolink:\n    enabled: true\n    endpoint: ${clusterEndpoint}\n    token: ${clusterToken}`;
  }

  async function copySnippet() {
    const s = clusterSnippet();
    if (!s) return;
    try {
      await navigator.clipboard.writeText(s);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      notifyError('Copy failed');
    }
  }

  async function createToken() {
    busy = true;
    try {
      const res = (await client.post('/siderolink/tokens', {
        label: label || null,
        expiresHours,
      })) as { token: string };
      lastToken = res.token;
      label = '';
      success('Join token created');
      await load();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create token');
    } finally {
      busy = false;
    }
  }
</script>

<div class="page">
  <h1>Siderolink inventory</h1>
  <p class="desc">
    Registration and join tokens for machines. WireGuard tunnel is managed automatically when
    <code>wireguard-tools</code> is installed on the TCS host.
  </p>

  <section class="card">
    <h2>Create join token</h2>
    <div class="form-row">
      <label>
        Label
        <input type="text" title="Optional label to identify this join token" bind:value={label} placeholder="lab-batch-1" />
      </label>
      <label class="num">
        Expires (hours)
        <input type="number" title="Hours until this join token expires" min="1" bind:value={expiresHours} />
      </label>
      <Button variant="primary" title="Create a join token machines use to register with Siderolink" onclick={createToken} disabled={busy}>Create token</Button>
    </div>
    {#if lastToken}
      <p class="token-out">
        New token (copy now): <code>{lastToken}</code>
      </p>
    {/if}
    <p class="muted">
      Machines register with <code>POST /api/siderolink/register</code> using
      <code>token</code>, <code>systemUuid</code>, and <code>publicKey</code>.
    </p>
  </section>

  <section class="card">
    <h2>Per-cluster tokens</h2>
    <p class="muted">
      Greenfield configs generated for a cluster automatically embed its token under
      <code>machine.siderolink</code>, so provisioned nodes dial in and form the tunnel on
      first boot. Rotate to issue a new token (old one stops working), revoke to remove it.
    </p>
    {#if clusters.length === 0}
      <p class="muted">No clusters yet — create one and its token will appear here.</p>
    {:else}
      <div class="form-row">
        <label>
          Cluster
          <select bind:value={selectedCluster}>
            {#each clusters as c (c.id)}
              <option value={c.id}>{c.name}</option>
            {/each}
          </select>
        </label>
        <Button variant="primary" title="Issue a new token for the selected cluster" onclick={rotateClusterToken} disabled={clusterTokenBusy || clusterTokenLoading}>
          {#if clusterToken}Rotate token{:else}Create token{/if}
        </Button>
        {#if clusterToken}
          <Button variant="ghost" title="Remove this cluster’s token" onclick={revokeClusterToken} disabled={clusterTokenBusy}>Revoke</Button>
        {/if}
      </div>
      {#if clusterTokenLoading}
        <p class="muted">Loading…</p>
      {:else if clusterToken}
        <div class="token-out">
          <div class="form-row">
            <label>
              Token
              <input readonly value={clusterToken} class="mono" />
            </label>
            <label>
              Endpoint
              <input readonly value={clusterEndpoint} class="mono" />
            </label>
          </div>
          <div class="form-row">
            <pre class="snippet">{clusterSnippet()}</pre>
            <Button variant="ghost" title="Copy the machine.siderolink snippet" onclick={copySnippet}>{copied ? 'Copied!' : 'Copy snippet'}</Button>
          </div>
        </div>
      {:else}
        <p class="muted">No token for this cluster yet. Click “Create token”.</p>
      {/if}
    {/if}
  </section>

  {#if loading}
    <Spinner />
  {:else}
    <section class="card">
      <h2>Join tokens</h2>
      {#if tokens.length === 0}
        <p class="muted">No tokens yet.</p>
      {:else}
        <table>
          <thead>
            <tr>
              <th>Token</th>
              <th>Label</th>
              <th>Expires</th>
              <th>Created</th>
            </tr>
          </thead>
          <tbody>
            {#each tokens as t}
              <tr>
                <td class="mono">{t.token}</td>
                <td>{t.label || '—'}</td>
                <td>{t.expiresAt || t.expires_at || 'never'}</td>
                <td>{t.createdAt || t.created_at || '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section class="card">
      <h2>Registered peers</h2>
      {#if peers.length === 0}
        <p class="muted">No peers registered.</p>
      {:else}
        <table>
          <thead>
            <tr>
              <th>System UUID</th>
              <th>Assigned IP</th>
              <th>Public key</th>
              <th>Last seen</th>
            </tr>
          </thead>
          <tbody>
            {#each peers as p}
              <tr>
                <td class="mono">{p.systemUuid || p.system_uuid}</td>
                <td>{p.assignedIp || p.assigned_ip}</td>
                <td class="mono short">{p.publicKey || p.public_key}</td>
                <td>{p.lastSeen || p.last_seen}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page h1 {
    margin: 0 0 0.35rem;
  }
  .desc {
    color: var(--tcs-text-muted);
    margin: 0 0 1.5rem;
    font-size: 0.9rem;
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
    align-items: flex-end;
    margin-bottom: 0.75rem;
  }
  .form-row label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: var(--tcs-text-muted);
  }
  .form-row label.num {
    width: 140px;
  }
  input {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.7rem;
    color: var(--tcs-text);
  }
  select {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.7rem;
    color: var(--tcs-text);
    min-width: 200px;
  }
  .snippet {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    color: var(--tcs-text);
    margin: 0;
    white-space: pre;
    flex: 1;
  }
  .token-out {
    font-size: 0.9rem;
    margin: 0.5rem 0;
  }
  .muted {
    color: var(--tcs-text-muted);
    font-size: 0.85rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.5rem 0.35rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
  }
  .short {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
