<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface AuditEntry {
    timestamp: string;
    user_email: string;
    action: string;
    resource: string;
    details: string;
  }

  interface AuditLogResponse {
    entries: AuditEntry[];
    total: number;
    page: number;
    per_page: number;
  }

  let entries = $state<AuditEntry[]>([]);
  let total = $state(0);
  let page = $state(1);
  let perPage = $state(50);
  let loading = $state(true);
  let error = $state('');
  let clearing = $state(false);

  let filterUser = $state('');
  let filterAction = $state('');
  let filterFrom = $state('');
  let filterTo = $state('');

  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  async function loadEntries(resetPage = false) {
    if (resetPage) page = 1;
    loading = true;
    error = '';
    try {
      const params = new URLSearchParams({
        page: page.toString(),
        per_page: perPage.toString(),
      });
      if (filterUser) params.set('user', filterUser);
      if (filterAction) params.set('action', filterAction);
      if (filterFrom) params.set('from', filterFrom);
      if (filterTo) params.set('to', filterTo);

      const data = await client.get(`/settings/audit-logs?${params}`) as AuditLogResponse;
      entries = data.entries;
      total = data.total;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load audit logs';
    } finally {
      loading = false;
    }
  }

  async function clearLogs() {
    if (!confirm('Clear all audit logs? This cannot be undone.')) return;
    clearing = true;
    try {
      await client.delete('/settings/audit-logs');
      entries = [];
      total = 0;
      success('Audit logs cleared');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to clear logs');
    } finally {
      clearing = false;
    }
  }

  function handleFilter() {
    loadEntries(true);
  }

  function resetFilters() {
    filterUser = '';
    filterAction = '';
    filterFrom = '';
    filterTo = '';
    loadEntries(true);
  }

  function formatTime(ts: string): string {
    return new Date(ts).toLocaleString();
  }

  onMount(() => {
    loadEntries();
    refreshTimer = setInterval(() => loadEntries(), 30000);
  });

  onDestroy(() => {
    if (refreshTimer) clearInterval(refreshTimer);
  });
</script>

<div class="audit-page">
  <div class="page-header">
    <div>
      <h1>Audit Logs</h1>
      <p class="description">Read-only trail of system actions. Auto-refreshes every 30s.</p>
    </div>
    <div class="header-actions">
      <span class="count">{total} total entries</span>
      <Button variant="danger" onclick={clearLogs} disabled={clearing}>
        {clearing ? 'Clearing...' : 'Clear Logs'}
      </Button>
    </div>
  </div>

  <div class="filters">
    <div class="filter-row">
      <input
        type="text"
        placeholder="Filter by user..."
        bind:value={filterUser}
        class="filter-input"
      />
      <input
        type="text"
        placeholder="Filter by action..."
        bind:value={filterAction}
        class="filter-input"
      />
      <input
        type="datetime-local"
        bind:value={filterFrom}
        class="filter-input"
      />
      <input
        type="datetime-local"
        bind:value={filterTo}
        class="filter-input"
      />
      <Button variant="secondary" onclick={handleFilter}>Apply</Button>
      <Button variant="ghost" onclick={resetFilters}>Reset</Button>
    </div>
  </div>

  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if entries.length === 0}
    <div class="empty-state">
      <p>No audit log entries found</p>
    </div>
  {:else}
    <div class="table-wrapper">
      <table class="data-table">
        <thead>
          <tr>
            <th>Timestamp</th>
            <th>User</th>
            <th>Action</th>
            <th>Resource</th>
            <th>Details</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as entry (entry.timestamp + entry.action)}
            <tr>
              <td class="timestamp">{formatTime(entry.timestamp)}</td>
              <td>{entry.user_email}</td>
              <td><span class="action-badge">{entry.action}</span></td>
              <td>{entry.resource}</td>
              <td class="details">{entry.details}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <div class="pagination">
      <Button
        variant="ghost"
        size="sm"
        disabled={page <= 1}
        onclick={() => { page--; loadEntries(); }}
      >
        Previous
      </Button>
      <span class="page-info">
        Page {page} (showing {entries.length} of {total})
      </span>
      <Button
        variant="ghost"
        size="sm"
        disabled={page * perPage >= total}
        onclick={() => { page++; loadEntries(); }}
      >
        Next
      </Button>
    </div>
  {/if}
</div>

<style>
  .audit-page h1 { margin: 0 0 0.25rem; }
  .description { color: var(--tcs-text-muted); margin: 0; font-size: 0.875rem; }
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1.5rem;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 1rem;
  }
  .count {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }

  .filters {
    margin-bottom: 1.5rem;
  }
  .filter-row {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .filter-input {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    color: var(--tcs-text);
    outline: none;
    font-size: 0.875rem;
  }
  .filter-input:focus {
    border-color: var(--tcs-primary);
  }

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

  .table-wrapper {
    overflow-x: auto;
  }
  .data-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.875rem;
  }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    position: sticky;
    top: 0;
    background: var(--tcs-background);
  }
  .data-table tr:hover {
    background: var(--tcs-surface-hover);
  }
  .data-table td.timestamp {
    white-space: nowrap;
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }
  .data-table td.details {
    max-width: 300px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.8rem;
  }

  .action-badge {
    font-size: 0.75rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: rgba(79, 139, 255, 0.15);
    color: var(--tcs-secondary);
    display: inline-block;
  }

  .pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    margin-top: 1rem;
    padding: 0.75rem 0;
  }
  .page-info {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
  }
</style>
