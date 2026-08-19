import { writable, derived, get } from 'svelte/store';
import { client } from '$lib/api/client';
import type { Cluster, DiscoveredCluster, ImportResult } from '$lib/api/types';

export type { Cluster, DiscoveredCluster, ImportResult };

export const clusters = writable<Cluster[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

export const clusterCount = derived(clusters, ($c) => $c.length);
export const runningClusters = derived(clusters, ($c) =>
  $c.filter((c) => c.status === 'running')
);

let pollTimer: ReturnType<typeof setInterval> | null = null;

/**
 * Load clusters from the API. In silent mode (polling) the spinner and error
 * banner are left alone; failures only surface when we have nothing to show,
 * so a transient poll failure never blanks the table.
 */
export async function loadClusters(silent = false): Promise<void> {
  if (!silent) {
    loading.set(true);
    error.set(null);
  }
  try {
    const data = (await client.get('/clusters')) as Cluster[];
    clusters.set(Array.isArray(data) ? data : []);
  } catch (e: unknown) {
    if (!silent || get(clusters).length === 0) {
      error.set(e instanceof Error ? e.message : 'Failed to load clusters');
    }
  } finally {
    if (!silent) loading.set(false);
  }
}

/** Poll the cluster list in the background so status changes appear live. */
export function startClustersPolling(intervalMs = 15000): void {
  stopClustersPolling();
  pollTimer = setInterval(() => void loadClusters(true), intervalMs);
}

export function stopClustersPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

export async function deleteCluster(id: string): Promise<void> {
  error.set(null);
  try {
    await client.delete(`/clusters/${id}`);
    clusters.update((items) => items.filter((c) => c.id !== id));
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to delete cluster');
    throw e;
  }
}

export async function previewImport(
  kubeconfig: string,
  name?: string
): Promise<DiscoveredCluster> {
  error.set(null);
  try {
    return (await client.post('/clusters/import/preview', {
      name: name || '',
      kubeconfig,
    })) as DiscoveredCluster;
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to preview import');
    throw e;
  }
}

export async function importCluster(
  name: string,
  kubeconfig: string,
  talosconfig?: string
): Promise<ImportResult> {
  error.set(null);
  try {
    const data = (await client.post('/clusters/import', {
      name,
      kubeconfig,
      talosconfig: talosconfig?.trim() || undefined,
    })) as ImportResult;
    clusters.update((items) => [data.cluster, ...items]);
    return data;
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to import cluster');
    throw e;
  }
}

export async function setClusterTalosconfig(
  clusterId: string,
  talosconfig: string
): Promise<void> {
  await client.put(`/clusters/${clusterId}/talosconfig`, { talosconfig });
}

export async function applyClusterConfig(
  clusterId: string,
  dryRun = false
): Promise<{ count: number; dryRun?: boolean }> {
  return (await client.post(`/clusters/${clusterId}/config/apply`, {
    dryRun,
  })) as { count: number; dryRun?: boolean };
}
