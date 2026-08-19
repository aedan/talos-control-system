<script lang="ts">
  import { onMount } from 'svelte';
  import {
    listNamespaces,
    listPods,
    listDeployments,
    listServices,
    listEvents,
    listNodes,
    getPod,
    getResource,
    type PodDetail,
  } from '$lib/api/k8s';
  import ResourceTree, { type ExplorerKind } from './ResourceTree.svelte';
  import DataTable, { type Column } from './DataTable.svelte';
  import YamlView from './YamlView.svelte';
  import LogPanel from './LogPanel.svelte';
  import TerminalPanel from './TerminalPanel.svelte';
  import Actions, { type Selection } from './Actions.svelte';

  let { clusterId }: { clusterId: string } = $props();

  let namespaces = $state<string[]>([]);
  let activeKind = $state<ExplorerKind>('pods');
  let activeNs = $state<string | undefined>(undefined);

  let rows = $state<Record<string, unknown>[]>([]);
  let loading = $state(false);
  let listError = $state('');

  let selected = $state<Record<string, unknown> | null>(null);
  let detailTab = $state<'yaml' | 'logs' | 'terminal' | 'actions'>('yaml');
  let podDetail = $state<PodDetail | null>(null);
  let rawDetail = $state<unknown>(null);
  let detailLoading = $state(false);

  const KIND_COLUMNS: Record<ExplorerKind, Column[]> = {
    pods: [
      { key: 'name', label: 'Name' },
      { key: 'namespace', label: 'Namespace' },
      { key: 'phase', label: 'Status' },
      { key: 'ready', label: 'Ready' },
      { key: 'restarts', label: 'Restarts' },
      { key: 'node', label: 'Node', wide: true },
      { key: 'ip', label: 'IP', wide: true },
      { key: 'age', label: 'Age' },
    ],
    deployments: [
      { key: 'name', label: 'Name' },
      { key: 'namespace', label: 'Namespace' },
      { key: 'ready', label: 'Ready' },
      { key: 'replicas', label: 'Replicas' },
      { key: 'available', label: 'Available', wide: true },
      { key: 'age', label: 'Age' },
    ],
    services: [
      { key: 'name', label: 'Name' },
      { key: 'namespace', label: 'Namespace' },
      { key: 'kind', label: 'Type' },
      { key: 'clusterIp', label: 'Cluster IP', wide: true },
      { key: 'ports', label: 'Ports' },
      { key: 'age', label: 'Age' },
    ],
    events: [
      { key: 'namespace', label: 'Namespace' },
      { key: 'name', label: 'Name' },
      { key: 'reason', label: 'Reason' },
      { key: 'object', label: 'Object' },
      { key: 'count', label: 'Count', wide: true },
      { key: 'lastSeen', label: 'Last Seen', wide: true },
    ],
    nodes: [
      { key: 'name', label: 'Name' },
      { key: 'status', label: 'Status' },
      { key: 'role', label: 'Role' },
      { key: 'kubernetesVersion', label: 'K8s', wide: true },
      { key: 'internalIp', label: 'Internal IP', wide: true },
      { key: 'age', label: 'Age' },
    ],
  };

  const columns = $derived(KIND_COLUMNS[activeKind]);
  const selection: Selection | null = $derived(
    selected
      ? { kind: activeKind, name: String(selected.name), ns: selected.namespace ? String(selected.namespace) : undefined }
      : null
  );
  const isPod = $derived(activeKind === 'pods');
  const containers = $derived(podDetail?.summary.containers ?? []);

  async function loadNamespaces() {
    try {
      const ns = (await listNamespaces(clusterId)) as { name: string }[];
      namespaces = ns.map((n) => n.name);
    } catch {
      /* non-fatal */
    }
  }

  async function loadList() {
    loading = true;
    listError = '';
    selected = null;
    podDetail = null;
    rawDetail = null;
    try {
      let data: Record<string, unknown>[];
      switch (activeKind) {
        case 'pods':
          data = (await listPods(clusterId, activeNs)) as unknown as Record<string, unknown>[];
          break;
        case 'deployments':
          data = (await listDeployments(clusterId, activeNs)) as unknown as Record<string, unknown>[];
          break;
        case 'services':
          data = (await listServices(clusterId, activeNs)) as unknown as Record<string, unknown>[];
          break;
        case 'events':
          data = (await listEvents(clusterId, activeNs)) as unknown as Record<string, unknown>[];
          break;
        case 'nodes':
          data = (await listNodes(clusterId)) as unknown as Record<string, unknown>[];
          break;
      }
      rows = data;
    } catch (e: unknown) {
      listError = e instanceof Error ? e.message : 'Failed to load resources';
      rows = [];
    } finally {
      loading = false;
    }
  }

  async function selectRow(row: Record<string, unknown>) {
    selected = row;
    detailTab = isPod ? 'yaml' : 'yaml';
    detailLoading = true;
    podDetail = null;
    rawDetail = null;
    const name = String(row.name);
    const ns = row.namespace ? String(row.namespace) : undefined;
    try {
      if (activeKind === 'pods') {
        podDetail = await getPod(clusterId, ns ?? 'default', name);
      } else {
        rawDetail = await getResource(clusterId, activeKind, name, ns);
      }
    } catch (e: unknown) {
      listError = e instanceof Error ? e.message : 'Failed to load detail';
    } finally {
      detailLoading = false;
    }
  }

  function detailYaml(): unknown {
    if (activeKind === 'pods') return podDetail?.yaml ?? null;
    return rawDetail ?? null;
  }

  $effect(() => {
    void activeKind;
    void activeNs;
    void loadList();
  });

  onMount(() => {
    void loadNamespaces();
  });
</script>

<div class="explorer">
  <div class="explorer-left">
    <ResourceTree
      {namespaces}
      {activeKind}
      {activeNs}
      onSelectKind={(k) => (activeKind = k)}
      onSelectNs={(ns) => (activeNs = ns)}
    />
  </div>

  <div class="explorer-middle">
    <div class="mid-head">
      <h2 class="mid-title">{activeKind}</h2>
      {#if activeNs !== undefined}
        <span class="mid-ns">{activeNs ?? 'all namespaces'}</span>
      {/if}
      <button class="refresh" onclick={loadList}>↻</button>
    </div>
    {#if listError}
      <div class="list-error">{listError}</div>
    {/if}
    <div class="mid-table">
      <DataTable
        {columns}
        {rows}
        {loading}
        selectedKey={selected ? String(selected.name) : null}
        onRowClick={selectRow}
        rowKey={(r) => `${r.namespace ?? ''}/${r.name}`}
      />
    </div>
  </div>

  <div class="explorer-right">
    {#if !selected}
      <div class="detail-empty">
        <p>Select a resource to inspect it.</p>
      </div>
    {:else}
      <div class="detail-head">
        <div class="detail-tabs">
          <button class:active={detailTab === 'yaml'} onclick={() => (detailTab = 'yaml')}>YAML</button>
          {#if isPod}
            <button class:active={detailTab === 'logs'} onclick={() => (detailTab = 'logs')}>Logs</button>
            <button class:active={detailTab === 'terminal'} onclick={() => (detailTab = 'terminal')}>Terminal</button>
          {/if}
          <button class:active={detailTab === 'actions'} onclick={() => (detailTab = 'actions')}>Actions</button>
        </div>
      </div>

      <div class="detail-body">
        {#if detailLoading}
          <div class="detail-empty"><p>Loading…</p></div>
        {:else if detailTab === 'yaml'}
          <YamlView value={detailYaml()} title={String(selected.name)} />
        {:else if detailTab === 'logs' && isPod}
          <LogPanel
            {clusterId}
            ns={String(selected.namespace ?? 'default')}
            name={String(selected.name)}
            containers={containers}
          />
        {:else if detailTab === 'terminal' && isPod}
          <TerminalPanel
            {clusterId}
            mode="exec"
            command={['sh', '-c', 'ls -la']}
            ns={String(selected.namespace ?? 'default')}
            name={String(selected.name)}
            containers={containers}
          />
        {:else if detailTab === 'actions'}
          <Actions {clusterId} {selection} onMutated={loadList} />
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .explorer {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) minmax(0, 1.1fr);
    gap: 0.75rem;
    height: calc(100vh - 220px);
    min-height: 480px;
  }
  .explorer-left,
  .explorer-middle,
  .explorer-right {
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .mid-head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }
  .mid-title {
    margin: 0;
    font-size: 1rem;
    text-transform: capitalize;
  }
  .mid-ns {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 4px;
    padding: 0.1rem 0.4rem;
  }
  .refresh {
    margin-left: auto;
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text-muted);
    cursor: pointer;
    padding: 0.2rem 0.5rem;
  }
  .refresh:hover {
    color: var(--tcs-text);
  }
  .mid-table {
    flex: 1;
    min-height: 0;
  }
  .list-error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: var(--tcs-error, #ef4444);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    font-size: 0.85rem;
    margin-bottom: 0.5rem;
  }
  .detail-head {
    margin-bottom: 0.5rem;
  }
  .detail-tabs {
    display: flex;
    gap: 0.25rem;
  }
  .detail-tabs button {
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text-muted);
    font-size: 0.82rem;
    padding: 0.35rem 0.75rem;
    cursor: pointer;
  }
  .detail-tabs button.active {
    background: rgba(79, 139, 255, 0.15);
    border-color: var(--tcs-secondary);
    color: var(--tcs-secondary);
    font-weight: 600;
  }
  .detail-body {
    flex: 1;
    min-height: 0;
  }
  .detail-empty {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tcs-text-muted);
    background: var(--tcs-surface);
    border: 1px dashed var(--tcs-border);
    border-radius: 8px;
  }
  .detail-empty p {
    margin: 0;
    font-size: 0.9rem;
  }

  @media (max-width: 1100px) {
    .explorer {
      grid-template-columns: 1fr;
      height: auto;
    }
    .explorer-left,
    .explorer-middle,
    .explorer-right {
      min-height: 320px;
    }
  }
</style>
