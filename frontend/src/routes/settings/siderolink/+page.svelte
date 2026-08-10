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

  async function load() {
    loading = true;
    try {
      const [p, t] = await Promise.all([
        client.get('/siderolink/peers') as Promise<any[]>,
        client.get('/siderolink/tokens') as Promise<any[]>,
      ]);
      peers = p || [];
      tokens = t || [];
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load Siderolink data');
    } finally {
      loading = false;
    }
  }

  onMount(load);

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
    Registration and join tokens for machines. WireGuard data path is not implemented yet — peers are
    inventory-only.
  </p>

  <section class="card">
    <h2>Create join token</h2>
    <div class="form-row">
      <label>
        Label
        <input type="text" bind:value={label} placeholder="lab-batch-1" />
      </label>
      <label class="num">
        Expires (hours)
        <input type="number" min="1" bind:value={expiresHours} />
      </label>
      <Button variant="primary" onclick={createToken} disabled={busy}>Create token</Button>
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
