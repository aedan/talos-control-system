<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  let agents = $state<any[]>([]);
  let tokens = $state<any[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let label = $state('');
  let expiresHours = $state(168);
  let lastToken = $state('');
  let lastLabel = $state('');

  async function load() {
    try {
      const [a, t] = await Promise.all([
        client.get('/proxy/agents') as Promise<any[]>,
        client.get('/proxy/tokens') as Promise<any[]>,
      ]);
      agents = a || [];
      tokens = t || [];
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to load proxy data');
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function createToken() {
    busy = true;
    try {
      const res = (await client.post('/proxy/tokens', {
        label: label || null,
        expiresHours,
      })) as { token: string };
      lastToken = res.token;
      lastLabel = label;
      label = '';
      success('OOB join token created');
      await load();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create token');
    } finally {
      busy = false;
    }
  }

  async function revokeToken(token: string) {
    if (!confirm(`Revoke join token ${token}? Connected agents will be dropped.`)) return;
    try {
      await client.delete(`/proxy/tokens/${encodeURIComponent(token)}`);
      success('Token revoked');
      await load();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to revoke token');
    }
  }

  function launchBrowserAgent(token: string, lbl?: string) {
    const params = new URLSearchParams({ token });
    if (lbl) params.set('label', lbl);
    window.open(`/proxy/agent?${params.toString()}`, '_blank', 'noopener');
  }
</script>

<div class="page">
  <h1>Remote OOB proxy</h1>
  <p class="desc">
    Join tokens for the OOB agent that dials out from a remote machine and relays Redfish BMC
    operations. Use the <strong>browser agent</strong> (launch from a token below) on any host that
    can reach the BMCs, or run the native <code>oob-agent</code> binary. Machines tagged with an
    agent id route their power/boot/ISO control through that agent; everything else stays on-network.
  </p>

  <section class="card">
    <h2>Create join token</h2>
    <div class="form-row">
      <label>
        Label
        <input type="text" title="Optional label to identify this OOB join token" bind:value={label} placeholder="dfw-site" />
      </label>
      <label class="num">
        Expires (hours)
        <input type="number" title="Hours until this OOB join token expires" min="1" bind:value={expiresHours} />
      </label>
      <Button variant="primary" title="Create a join token the oob-agent uses to connect" onclick={createToken} disabled={busy}>Create token</Button>
    </div>
    {#if lastToken}
      <p class="token-out">
        New token (copy now): <code>{lastToken}</code>
        <Button variant="secondary" size="sm" title="Open the browser OOB agent in a new tab using this token" onclick={() => launchBrowserAgent(lastToken, lastLabel)}>Launch browser agent</Button>
      </p>
    {/if}
    <p class="muted">
      Or run the native agent with
      <code>oob-agent --server wss://&lt;tcs&gt;/api/proxy/tunnel --token pxj_…</code>.
    </p>
  </section>

  {#if loading}
    <Spinner />
  {:else}
    <section class="card">
      <h2>Connected agents</h2>
      {#if agents.length === 0}
        <p class="muted">No agents connected.</p>
      {:else}
        <table>
          <thead>
            <tr>
              <th>Agent id</th>
              <th>Label</th>
              <th>Caps</th>
              <th>Connected for</th>
            </tr>
          </thead>
          <tbody>
            {#each agents as a}
              <tr>
                <td class="mono">{a.agentId}</td>
                <td>{a.label || '—'}</td>
                <td>{(a.caps || []).join(', ') || '—'}</td>
                <td>{a.connectedForSecs}s</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

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
              <th>Agent</th>
            </tr>
          </thead>
          <tbody>
            {#each tokens as t}
              <tr>
                <td class="mono">{t.token}</td>
                <td>{t.label || '—'}</td>
                <td>{t.expiresAt || 'never'}</td>
                <td>{t.createdAt || '—'}</td>
                <td>
                  <Button variant="secondary" size="sm" title="Open the browser OOB agent in a new tab; it connects using this token" onclick={() => launchBrowserAgent(t.token, t.label)}>Launch browser agent</Button>
                </td>
                <td>
                  <Button variant="ghost" size="sm" title="Revoke this join token" onclick={() => revokeToken(t.token)}>Revoke</Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page h1 { margin: 0 0 0.35rem; }
  .desc { color: var(--tcs-text-muted); margin: 0 0 1.5rem; font-size: 0.9rem; }
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1.25rem;
    margin-bottom: 1.25rem;
  }
  .card h2 { margin: 0 0 1rem; font-size: 1.05rem; }
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
  .form-row label.num { width: 140px; }
  input {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.7rem;
    color: var(--tcs-text);
  }
  .token-out { font-size: 0.9rem; margin: 0.5rem 0; }
  .muted { color: var(--tcs-text-muted); font-size: 0.85rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  th, td { text-align: left; padding: 0.5rem 0.35rem; border-bottom: 1px solid var(--tcs-border); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.78rem; }
</style>
