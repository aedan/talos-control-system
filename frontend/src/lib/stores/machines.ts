import { writable } from 'svelte/store';
import { client } from '$lib/api/client';
import type { Machine } from '$lib/api/types';

export type { Machine };

export const machines = writable<Machine[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

export async function loadMachines(): Promise<void> {
  loading.set(true);
  error.set(null);
  try {
    const data = (await client.get('/machines')) as Machine[];
    machines.set(Array.isArray(data) ? data : []);
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to load machines');
  } finally {
    loading.set(false);
  }
}

export async function loadClusterMachines(clusterId: string): Promise<Machine[]> {
  const data = (await client.get(`/clusters/${clusterId}/machines`)) as Machine[];
  return Array.isArray(data) ? data : [];
}
