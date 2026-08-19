<script lang="ts">
  export interface Column {
    key: string;
    label: string;
    /** When true, the column is only shown in wide mode. */
    wide?: boolean;
    mono?: boolean;
  }

  let {
    columns,
    rows,
    wide = false,
    selectedKey = null,
    onRowClick,
    emptyMessage = 'No resources',
    loading = false,
    rowKey,
  }: {
    columns: Column[];
    rows: Record<string, unknown>[];
    wide?: boolean;
    selectedKey?: string | null;
    onRowClick?: (row: Record<string, unknown>) => void;
    emptyMessage?: string;
    loading?: boolean;
    rowKey?: (row: Record<string, unknown>) => string;
  } = $props();

  const visibleColumns = $derived(
    wide ? columns : columns.filter((c) => !c.wide)
  );

  function keyOf(row: Record<string, unknown>, i: number): string {
    if (rowKey) return rowKey(row);
    return String(row['name'] ?? row['namespace'] ?? i);
  }

  function cell(row: Record<string, unknown>, col: Column): string {
    const v = row[col.key];
    if (v == null) return '—';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }
</script>

<div class="dt-wrap">
  {#if loading}
    <div class="dt-empty">Loading…</div>
  {:else if rows.length === 0}
    <div class="dt-empty">{emptyMessage}</div>
  {:else}
    <div class="dt-scroll">
      <table class="dt">
        <thead>
          <tr>
            {#each visibleColumns as col (col.key)}
              <th class:mono={col.mono}>{col.label}</th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as row, i (keyOf(row, i))}
            {@const k = keyOf(row, i)}
            <tr
              class="clickable"
              class:selected={selectedKey === k}
              onclick={onRowClick ? () => onRowClick(row) : undefined}
            >
              {#each visibleColumns as col (col.key)}
                <td class:mono={col.mono}>{cell(row, col)}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .dt-wrap {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .dt-scroll {
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  .dt {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  .dt th,
  .dt td {
    text-align: left;
    padding: 0.55rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
    white-space: nowrap;
  }
  .dt th {
    position: sticky;
    top: 0;
    background: var(--tcs-surface);
    color: var(--tcs-text-muted);
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    z-index: 1;
  }
  .dt td.mono,
  .dt th.mono {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }
  .dt tr.clickable {
    cursor: pointer;
  }
  .dt tr.clickable:hover {
    background: var(--tcs-surface-hover);
  }
  .dt tr.selected {
    background: rgba(79, 139, 255, 0.12);
  }
  .dt-empty {
    padding: 2rem;
    text-align: center;
    color: var(--tcs-text-muted);
    font-size: 0.9rem;
  }
</style>
