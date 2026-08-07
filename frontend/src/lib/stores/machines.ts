import { writable, derived } from 'svelte/store';
import { client } from '$lib/api/client';

export interface Machine {
  id: string;
  systemUuid: string;
  machineType: 'controlplane' | 'worker';
  clusterId: string | null;
  clusterName: string | null;
  status: 'pending' | 'booting' | 'installing' | 'configuring' | 'running' | 'destroying';
  talosVersion: string;
  kubernetesVersion: string | null;
  hostname: string | null;
  arch: string;
  memoryBytes: number;
  cpuCores: number;
  diskBytes: number;
  ip: string | null;
  createdAt: string;
}

export const machines = writable<Machine[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

export const pendingMachines = derived(machines, ($m) => $m.filter((m) => m.status === 'pending'));
export const runningMachines = derived(machines, ($m) => $m.filter((m) => m.status === 'running'));

export async function loadMachines(): Promise<void> {
  loading.set(true);
  error.set(null);
  try {
    const data = await client.get('/machines') as Machine[];
    machines.set(data);
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to load machines');
  } finally {
    loading.set(false);
  }
}
