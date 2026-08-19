<script lang="ts">
  export type ExplorerKind = 'pods' | 'deployments' | 'services' | 'events' | 'nodes';

  const KINDS: { id: ExplorerKind; label: string; namespaced: boolean }[] = [
    { id: 'pods', label: 'Pods', namespaced: true },
    { id: 'deployments', label: 'Deployments', namespaced: true },
    { id: 'services', label: 'Services', namespaced: true },
    { id: 'events', label: 'Events', namespaced: true },
    { id: 'nodes', label: 'Nodes', namespaced: false },
  ];

  let {
    namespaces = [],
    activeKind,
    activeNs,
    onSelectKind,
    onSelectNs,
  }: {
    namespaces: string[];
    activeKind: ExplorerKind;
    activeNs: string | undefined;
    onSelectKind: (k: ExplorerKind) => void;
    onSelectNs: (ns: string | undefined) => void;
  } = $props();

  const activeNamespaced = $derived(KINDS.find((k) => k.id === activeKind)?.namespaced ?? true);
</script>

<aside class="tree">
  <section class="tree-section">
    <h3 class="tree-head">Resources</h3>
    <ul class="kind-list">
      {#each KINDS as k (k.id)}
        <li>
          <button
            class="kind-btn"
            class:active={activeKind === k.id}
            onclick={() => {
              onSelectKind(k.id);
              if (!k.namespaced) onSelectNs(undefined);
            }}
          >
            {k.label}
          </button>
        </li>
      {/each}
    </ul>
  </section>

  {#if activeNamespaced}
    <section class="tree-section">
      <h3 class="tree-head">Namespaces</h3>
      <ul class="ns-list">
        <li>
          <button
            class="ns-btn"
            class:active={activeNs === undefined}
            onclick={() => onSelectNs(undefined)}
          >
            all
          </button>
        </li>
        {#each namespaces as ns (ns)}
          <li>
            <button
              class="ns-btn"
              class:active={activeNs === ns}
              onclick={() => onSelectNs(ns)}
            >
              {ns}
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</aside>

<style>
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    padding: 0.75rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow-y: auto;
    min-height: 0;
  }
  .tree-head {
    margin: 0 0 0.5rem;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--tcs-text-muted);
    font-weight: 700;
  }
  .kind-list,
  .ns-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .kind-btn,
  .ns-btn {
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 5px;
    padding: 0.4rem 0.6rem;
    color: var(--tcs-text);
    font-size: 0.85rem;
    cursor: pointer;
  }
  .ns-btn {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }
  .kind-btn:hover,
  .ns-btn:hover {
    background: var(--tcs-surface-hover);
  }
  .kind-btn.active {
    background: rgba(79, 139, 255, 0.15);
    color: var(--tcs-secondary);
    font-weight: 600;
  }
  .ns-btn.active {
    background: rgba(79, 139, 255, 0.12);
    color: var(--tcs-text);
  }
</style>
