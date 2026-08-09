<script lang="ts">
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface MachineClass {
    id: string;
    name: string;
    description: string;
    minCpu: number;
    minMemory: number;
    minDisk: number;
    arch: string;
    secureBoot: boolean;
    allowedRoles: string[];
    machineCount: number;
    createdAt: string;
    updatedAt: string;
  }
  
  let machineClass = $state<MachineClass | null>(null);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      // Placeholder - API not yet implemented
      machineClass = {
        id: $page.params.id ?? '',
        name: 'Standard Worker',
        description: 'Default worker node profile',
        minCpu: 4,
        minMemory: 8 * 1024 * 1024 * 1024,
        minDisk: 40 * 1024 * 1024 * 1024,
        arch: 'x86_64',
        secureBoot: true,
        allowedRoles: ['worker'],
        machineCount: 0,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString()
      };
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load machine class';
    } finally {
      loading = false;
    }
  });
  
  function formatBytes(bytes: number): string {
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
</script>

<div class="class-detail">
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if machineClass}
    <div class="detail-header">
      <h1>{machineClass.name}</h1>
      <p class="description">{machineClass.description}</p>
    </div>
    
    <div class="info-grid">
      <div class="info-section">
        <h2>Requirements</h2>
        <div class="info-row">
          <span class="label">Min CPU</span>
          <span class="value">{machineClass.minCpu} cores</span>
        </div>
        <div class="info-row">
          <span class="label">Min Memory</span>
          <span class="value">{formatBytes(machineClass.minMemory)}</span>
        </div>
        <div class="info-row">
          <span class="label">Min Disk</span>
          <span class="value">{formatBytes(machineClass.minDisk)}</span>
        </div>
        <div class="info-row">
          <span class="label">Architecture</span>
          <span class="value">{machineClass.arch}</span>
        </div>
      </div>
      
      <div class="info-section">
        <h2>Policies</h2>
        <div class="info-row">
          <span class="label">Secure Boot</span>
          <span class="value">{machineClass.secureBoot ? 'Required' : 'Optional'}</span>
        </div>
        <div class="info-row">
          <span class="label">Allowed Roles</span>
          <span class="value">
            {#each machineClass.allowedRoles as r}
              <span class="role-tag">{r}</span>
            {/each}
          </span>
        </div>
        <div class="info-row">
          <span class="label">Machine Count</span>
          <span class="value">{machineClass.machineCount}</span>
        </div>
      </div>
    </div>
    
    <div class="coming-soon-notice">
      <p>Machine class management is under development. Editing capabilities will be available in the next release.</p>
    </div>
  {/if}
</div>

<style>
  .class-detail h1 { margin: 0; }
  .description {
    color: var(--tcs-text-muted);
    margin: 0.5rem 0 2rem;
  }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  
  .detail-header { margin-bottom: 2rem; }
  
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1.5rem;
    margin-bottom: 2rem;
  }
  
  .info-section {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
  }
  
  .info-section h2 {
    margin: 0 0 1rem;
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  
  .info-row {
    display: flex;
    justify-content: space-between;
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--tcs-border);
    font-size: 0.875rem;
  }
  
  .info-row:last-child { border-bottom: none; }
  
  .label { color: var(--tcs-text-muted); }
  
  .value { font-weight: 500; }
  
  .role-tag {
    font-size: 0.7rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    margin-right: 0.25rem;
  }
  
  .coming-soon-notice {
    background: rgba(245, 158, 11, 0.1);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-warning);
    font-size: 0.875rem;
  }
  
  .coming-soon-notice p { margin: 0; }
</style>
