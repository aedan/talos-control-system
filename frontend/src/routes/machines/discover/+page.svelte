<script lang="ts">
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import { goto } from '$app/navigation';
  import Button from '$lib/components/Button.svelte';

  // ── Core connection settings (persisted to localStorage) ──────────────
  const LS_KEY = 'tcs_core_discovery';
  function loadSettings(): { baseUrl: string; path: string } {
    try {
      const raw = localStorage.getItem(LS_KEY);
      if (raw) return { baseUrl: '', path: '', ...JSON.parse(raw) };
    } catch { /* ignore */ }
    return { baseUrl: '', path: '/api' };
  }
  let baseUrl = $state(loadSettings().baseUrl);
  let path = $state(loadSettings().path);
  $effect(() => {
    try { localStorage.setItem(LS_KEY, JSON.stringify({ baseUrl, path })); } catch { /* ignore */ }
  });

  // ── Discovery state ────────────────────────────────────────────────────
  let rawJson = $state('');
  let fetching = $state(false);
  let fetchError = $state('');

  // ── Normalized inventory ───────────────────────────────────────────────
  let content = $state('');
  let normalizeError = $state('');

  // ── Import options ─────────────────────────────────────────────────────
  let agents = $state<any[]>([]);
  let proxyId = $state('');
  let createCluster = $state(true);
  let createClusterName = $state('');
  let clusterId = $state('');
  let clusters = $state<Array<{ id: string; name: string }>>([]);
  let preview = $state<any>(null);
  let result = $state<any>(null);
  let busy = $state(false);

  async function loadClusters() {
    try { clusters = ((await client.get('/clusters')) as any[]) || []; } catch { clusters = []; }
  }
  async function loadAgents() {
    try { agents = ((await client.get('/proxy/agents')) as any[]) || []; } catch { agents = []; }
  }
  loadClusters();
  loadAgents();

  function coreUrl(): string {
    const base = baseUrl.trim().replace(/\/+$/, '');
    const p = path.trim();
    if (!base) return '';
    return p ? `${base}${p.startsWith('/') ? p : '/' + p}` : base;
  }

  async function doFetch() {
    const url = coreUrl();
    if (!url) { fetchError = 'Enter the Core base URL.'; return; }
    fetching = true;
    fetchError = '';
    rawJson = '';
    try {
      const res = await fetch(url, {
        credentials: 'include',
        headers: { Accept: 'application/json' },
      });
      const text = await res.text();
      if (!res.ok) {
        fetchError = `Core returned ${res.status} ${res.statusText}`;
        rawJson = text;
      } else {
        rawJson = text;
        // Try to pretty-print for the debug panel.
        try { rawJson = JSON.stringify(JSON.parse(text), null, 2); } catch { /* keep raw */ }
        doNormalize();
      }
    } catch (e: unknown) {
      fetchError = e instanceof Error ? e.message : 'Fetch failed (network/CORS)';
    } finally {
      fetching = false;
    }
  }

  // ── Best-effort Core → inventory normalization ─────────────────────────
  // Core's exact device schema is unknown, so this auto-detects the largest
  // array of objects in the response and maps candidate field names. The
  // result lands in an editable YAML buffer for the operator to fix before
  // import.
  function findDeviceArray(node: any, depth = 0): any[] {
    if (depth > 6) return [];
    if (Array.isArray(node)) {
      const objs = node.filter((x) => x && typeof x === 'object' && !Array.isArray(x));
      return objs.length >= 1 ? objs : [];
    }
    if (node && typeof node === 'object') {
      let best: any[] = [];
      for (const v of Object.values(node)) {
        const found = findDeviceArray(v, depth + 1);
        if (found.length > best.length) best = found;
      }
      return best;
    }
    return [];
  }

  function pick(obj: Record<string, any>, candidates: string[]): any {
    const lower: Record<string, any> = {};
    for (const k of Object.keys(obj)) lower[k.toLowerCase()] = obj[k];
    for (const c of candidates) {
      if (lower[c] !== undefined && lower[c] !== null && lower[c] !== '') return lower[c];
    }
    return undefined;
  }

  function macFrom(obj: Record<string, any>): string {
    const v = pick(obj, ['mac', 'macaddress', 'mac_address', 'primarymac', 'hwmac', 'netmac', 'nicmac']);
    return v ? String(v) : '';
  }
  function bmcAddressFrom(obj: Record<string, any>): string {
    return (
      pick(obj, ['bmcaddress', 'bmc_address', 'iloaddress', 'ilo_address', 'oobaddress', 'oob_address', 'mgmtaddress', 'mgmt_address', 'redfishaddress', 'ipmiaddress']) ||
      ''
    );
  }

  function toYaml(machines: any[], clusterName: string): string {
    const lines: string[] = [];
    if (clusterName) {
      lines.push('cluster:', `  name: ${clusterName}`);
    }
    lines.push('machines:');
    for (const m of machines) {
      lines.push(`  - hostname: ${m.hostname || ''}`);
      if (m.role) lines.push(`    role: ${m.role}`);
      if (m.mac) lines.push(`    mac: ${m.mac}`);
      if (m.address) lines.push(`    address: ${m.address}`);
      if (m.systemUuid) lines.push(`    systemUuid: ${m.systemUuid}`);
      if (m.bmcAddress) {
        lines.push('    bmc:');
        lines.push(`      address: ${m.bmcAddress}`);
        if (m.bmcUsername) lines.push(`      username: ${m.bmcUsername}`);
        if (m.bmcPassword) lines.push(`      password: ${m.bmcPassword}`);
        lines.push('      type: auto');
      }
    }
    return lines.join('\n');
  }

  function doNormalize() {
    normalizeError = '';
    content = '';
    if (!rawJson.trim()) return;
    let data: any;
    try {
      data = JSON.parse(rawJson);
    } catch (e: unknown) {
      normalizeError = e instanceof Error ? e.message : 'Raw response is not valid JSON.';
      return;
    }
    const devices = findDeviceArray(data);
    if (devices.length === 0) {
      normalizeError = 'No array of device objects found in the response. Paste inventory YAML below manually.';
      return;
    }
    const machines = devices.map((d) => {
      const obj = d as Record<string, any>;
      return {
        hostname: String(pick(obj, ['hostname', 'name', 'nodename', 'node_name', 'id', 'uuid']) ?? ''),
        role: 'worker',
        mac: macFrom(obj),
        address: String(pick(obj, ['address', 'ip', 'ipv4', 'ipaddress', 'mgmtip', 'mgmt_ip']) ?? ''),
        systemUuid: String(pick(obj, ['systemuuid', 'system_uuid', 'uuid', 'serialnumber', 'serial']) ?? ''),
        bmcAddress: bmcAddressFrom(obj),
        bmcUsername: String(pick(obj, ['bmcusername', 'ilo_username', 'ipmiuser']) ?? ''),
        bmcPassword: String(pick(obj, ['bmcpassword', 'ilo_password', 'ipmipass']) ?? ''),
      };
    });
    content = toYaml(machines, '');
  }

  async function doPreview() {
    busy = true;
    result = null;
    try {
      preview = await client.post('/machines/import/preview', { format: 'yaml', content });
      success(`Preview: ${(preview as any).machines?.length || 0} rows`);
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Preview failed');
    } finally {
      busy = false;
    }
  }

  async function doImport() {
    busy = true;
    try {
      result = await client.post('/machines/import', {
        format: 'yaml',
        content,
        createCluster: createCluster && !clusterId,
        createClusterName: createClusterName || undefined,
        clusterId: clusterId || undefined,
        proxyId: proxyId || undefined,
        upsertByMac: true,
      });
      success(
        `Imported: ${result.created} created, ${result.updated} updated` +
          (result.errors?.length ? `, ${result.errors.length} row errors` : '')
      );
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Import failed');
    } finally {
      busy = false;
    }
  }
</script>

<div class="import-page">
  <div class="page-header">
    <h1>Discover from Rackspace Core</h1>
    <a href="/machines/import"><Button variant="ghost" size="sm" title="Use the manual CSV/YAML import page instead">Manual import</Button></a>
  </div>
  <p class="hint">
    Pulls the device list straight from your Rackspace Core session in the browser (cookies are
    sent with the request). The result is normalized to inventory YAML you can edit, then imported
    — optionally tagging machines to route their BMC control through a connected OOB agent.
  </p>

  <section class="card">
    <h2>Core connection</h2>
    <div class="row">
      <label class="grow">
        Base URL
        <input type="text" title="Rackspace Core base URL, e.g. https://ws.core.rackspace.com" bind:value={baseUrl} placeholder="https://ws.core.rackspace.com" />
      </label>
      <label>
        Path
        <input type="text" title="Device-list endpoint path relative to the base URL" bind:value={path} placeholder="/api" />
      </label>
      <Button variant="secondary" title="Fetch the device list from Core using your browser session" onclick={doFetch} disabled={fetching}>Fetch</Button>
    </div>
    {#if fetchError}
      <p class="err">{fetchError}</p>
    {/if}
    {#if rawJson}
      <details class="raw">
        <summary>Raw JSON response</summary>
        <pre>{rawJson}</pre>
      </details>
    {/if}
  </section>

  <section class="card">
    <h2>Normalized inventory</h2>
    {#if normalizeError}
      <p class="err">{normalizeError}</p>
    {/if}
    <textarea title="Normalized machine inventory (edit before importing)" bind:value={content} rows="14" spellcheck="false" placeholder="Fetched devices will be normalized here as YAML."></textarea>
    <div class="row">
      <label>
        OOB agent (proxy)
        <select title="Route imported machines' BMC operations through this connected OOB agent" bind:value={proxyId}>
          <option value="">— none (on-network) —</option>
          {#each agents as a (a.agentId)}
            <option value={a.agentId}>{a.agentId}{a.label ? ` (${a.label})` : ''}</option>
          {/each}
        </select>
      </label>
      <label class="check">
        <input type="checkbox" title="Create a cluster from the YAML name when no existing cluster is selected" bind:checked={createCluster} disabled={!!clusterId} />
        Create cluster from YAML name
      </label>
      <label>
        Cluster name override
        <input type="text" title="Override the name of the cluster to create" bind:value={createClusterName} placeholder="optional" />
      </label>
      <label>
        Attach to existing cluster
        <select title="Attach imported machines to an existing cluster" bind:value={clusterId}>
          <option value="">— none / create —</option>
          {#each clusters as c (c.id)}
            <option value={c.id}>{c.name}</option>
          {/each}
        </select>
      </label>
    </div>
    <div class="actions">
      <Button variant="secondary" title="Parse the inventory and show a preview without importing" onclick={doPreview} disabled={busy || !content}>Preview</Button>
      <Button variant="primary" title="Import the machines into TCS" onclick={doImport} disabled={busy || !content}>Import</Button>
    </div>
  </section>

  {#if preview}
    <section class="card">
      <h2>Preview</h2>
      {#if preview.errors?.length}
        <ul class="errs">
          {#each preview.errors as e}
            <li>Row {e.index}: {e.message}</li>
          {/each}
        </ul>
      {/if}
      <table class="data-table">
        <thead>
          <tr>
            <th>#</th><th>Hostname</th><th>Role</th><th>MAC</th><th>Address</th><th>BMC</th>
          </tr>
        </thead>
        <tbody>
          {#each preview.machines || [] as m}
            <tr>
              <td>{m.index}</td>
              <td>{m.hostname || '—'}</td>
              <td>{m.role}</td>
              <td class="mono">{m.mac || '—'}</td>
              <td class="mono">{m.address || '—'}</td>
              <td class="mono">{m.bmcAddress || '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  {#if result}
    <section class="card">
      <h2>Result</h2>
      <p>
        Created <strong>{result.created}</strong>, updated <strong>{result.updated}</strong>
        {#if result.clusterId}
          · cluster <code>{result.clusterId}</code>
        {/if}
      </p>
      {#if result.errors?.length}
        <ul class="errs">
          {#each result.errors as e}
            <li>Row {e.index}: {e.message}</li>
          {/each}
        </ul>
      {/if}
      {#if result.clusterId}
        <Button variant="primary" size="sm" title="Open the cluster these machines were imported into" onclick={() => goto(`/clusters/${result.clusterId}`)}>Open cluster</Button>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page-header { display: flex; justify-content: space-between; align-items: center; gap: 1rem; }
  .hint { color: var(--tcs-text-muted); max-width: 52rem; }
  .row { display: flex; flex-wrap: wrap; gap: 1rem; margin: 1rem 0; align-items: end; }
  label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; }
  label.grow { flex: 1; min-width: 260px; }
  label.check { flex-direction: row; align-items: center; gap: 0.5rem; }
  textarea, input, select {
    padding: 0.4rem 0.5rem; border-radius: 6px; border: 1px solid var(--tcs-border);
    background: var(--tcs-background); color: var(--tcs-text);
  }
  textarea { width: 100%; font-family: ui-monospace, monospace; font-size: 0.8rem; min-height: 14rem; }
  .actions { display: flex; gap: 0.5rem; margin: 1rem 0; }
  .card {
    background: var(--tcs-surface); border: 1px solid var(--tcs-border);
    border-radius: 8px; padding: 1rem; margin-top: 1rem;
  }
  .card h2 { margin: 0 0 0.75rem; font-size: 1rem; }
  .data-table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  .data-table th, .data-table td { text-align: left; padding: 0.35rem 0.4rem; border-bottom: 1px solid var(--tcs-border); }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .errs { color: var(--tcs-error); font-size: 0.85rem; }
  .err { color: var(--tcs-error); font-size: 0.85rem; margin: 0.5rem 0; }
  .raw summary { cursor: pointer; font-size: 0.85rem; color: var(--tcs-text-muted); }
  .raw pre {
    font-family: ui-monospace, monospace; font-size: 0.75rem; max-height: 20rem; overflow: auto;
    background: var(--tcs-background); border: 1px solid var(--tcs-border); border-radius: 6px; padding: 0.6rem;
  }
</style>
