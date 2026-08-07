import { writable, derived } from 'svelte/store';
import { client } from '$lib/api/client';

export interface Cluster {
  id: string;
  name: string;
  controlPlaneVersion: string;
  talosVersion: string;
  status: 'unknown' | 'scalingUp' | 'scalingDown' | 'running' | 'destroying';
  ready: boolean;
  available: boolean;
  controlPlaneNodes: number;
  workerNodes: number;
  createdAt: string;
}

export const clusters = writable<Cluster[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

export const clusterCount = derived(clusters, ($c) => $c.length);
export const runningClusters = derived(clusters, ($c) => $c.filter((c) => c.status === 'running'));

export async function loadClusters(): Promise<void> {
  loading.set(true);
  error.set(null);
  try {
    const data = await client.get('/clusters') as Cluster[];
    clusters.set(data);
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to load clusters');
  } finally {
    loading.set(false);
  }
}

export async function deleteCluster(id: string): Promise<void> {
  error.set(null);
  try {
    await client.delete(`/clusters/${id}`);
    clusters.update((items) => items.filter((c) => c.id !== id));
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to delete cluster');
  }
}
