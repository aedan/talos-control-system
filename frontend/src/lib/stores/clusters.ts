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

export interface DiscoveredNode {
  name: string;
  internalIp: string;
  kubernetesVersion: string;
  talosVersion: string;
  role: string;
  osImage: string;
}

export interface DiscoveredCluster {
  name: string;
  server: string;
  kubernetesVersion: string;
  talosVersion: string;
  controlPlaneNodes: DiscoveredNode[];
  workerNodes: DiscoveredNode[];
  isTalos: boolean;
}

export interface ImportResult {
  cluster: Cluster;
  machinesImported: number;
}

export async function previewImport(kubeconfig: string, name?: string): Promise<DiscoveredCluster> {
  error.set(null);
  try {
    const data = await client.post('/clusters/import/preview', {
      name: name || '',
      kubeconfig,
    });
    return data as DiscoveredCluster;
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
    const data = await client.post('/clusters/import', {
      name,
      kubeconfig,
      talosconfig: talosconfig?.trim() || undefined,
    });
    const result = data as ImportResult;
    clusters.update((items) => [result.cluster, ...items]);
    return result;
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

export async function applyClusterConfig(clusterId: string): Promise<{ applied_to: string[]; count: number }> {
  return (await client.post(`/clusters/${clusterId}/config/apply`, {})) as {
    applied_to: string[];
    count: number;
  };
}
