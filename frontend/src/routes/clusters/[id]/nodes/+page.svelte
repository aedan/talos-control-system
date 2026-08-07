<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface Node {
    name: string;
    status: string;
    role: string;
    version: string;
    internalIp: string;
    osImage: string;
    kernelVersion: string;
    containerRuntime: string;
    cpu: string;
    memory: string;
    conditions: Array<{ type: string; status: string; reason: string }>;
  }
  
  let nodes = $state<Node[]>([]);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      nodes = await client.get(`/clusters/${$page.params.id}/nodes`) as Node[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load nodes';
    } finally {
      loading = false;
    }
  });
  
  function isReady(node: Node): boolean {
    return node.conditions?.some(c => c.type === 'Ready' && c.status === 'True') ?? false;
  }
</script>

<div class="nodes-page">
  <h1>Nodes</h1>
  
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if nodes.length === 0}
    <div class="empty-state">
      <p>No nodes found in this cluster</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Status</th>
          <th>Role</th>
          <th>Version</th>
          <th>IP</th>
          <th>CPU</th>
          <th>Memory</th>
          <th>Runtime</th>
        </tr>
      </thead>
      <tbody>
        {#each nodes as node (node.name)}
          <tr>
            <td class="node-name">{node.name}</td>
            <td>
              <span class="status-dot {isReady(node) ? 'ready' : 'not-ready'}"></span>
              {isReady(node) ? 'Ready' : 'NotReady'}
            </td>
            <td>{node.role}</td>
            <td>{node.version}</td>
            <td>{node.internalIp}</td>
            <td>{node.cpu}</td>
            <td>{node.memory}</td>
            <td>{node.containerRuntime}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .nodes-page h1 { margin: 0 0 1.5rem; }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty-state {
    text-align: center;
    padding: 3rem;
    color: var(--tcs-text-muted);
  }
  
  .data-table {
    width: 100%;
    border-collapse: collapse;
  }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.875rem;
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .data-table tr:hover {
    background: var(--tcs-surface-hover);
  }
  
  .node-name {
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 0.8rem;
  }
  
  .status-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 0.4rem;
  }
  .status-dot.ready {
    background: var(--tcs-success);
    box-shadow: 0 0 6px var(--tcs-success);
  }
  .status-dot.not-ready {
    background: var(--tcs-error);
    box-shadow: 0 0 6px var(--tcs-error);
  }
</style>
