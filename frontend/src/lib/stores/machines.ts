import { writable, get } from 'svelte/store';
import { client } from '$lib/api/client';
import type { Machine } from '$lib/api/types';

export type { Machine };

export const machines = writable<Machine[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

let pollTimer: ReturnType<typeof setInterval> | null = null;

/**
 * Load machines from the API. In silent mode (polling) the spinner and
 * error banner are left alone; failures only surface when we have nothing
 * to show, so a transient poll failure never blanks the table.
 */
export async function loadMachines(silent = false): Promise<void> {
  if (!silent) {
    loading.set(true);
    error.set(null);
  }
  try {
    const data = (await client.get('/machines')) as Machine[];
    machines.set(Array.isArray(data) ? data : []);
  } catch (e: unknown) {
    if (!silent || get(machines).length === 0) {
      error.set(e instanceof Error ? e.message : 'Failed to load machines');
    }
  } finally {
    if (!silent) loading.set(false);
  }
}

/** Poll the machine list in the background so status changes appear live. */
export function startMachinesPolling(intervalMs = 15000): void {
  stopMachinesPolling();
  pollTimer = setInterval(() => void loadMachines(true), intervalMs);
}

export function stopMachinesPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

export async function loadClusterMachines(clusterId: string): Promise<Machine[]> {
  const data = (await client.get(`/clusters/${clusterId}/machines`)) as Machine[];
  return Array.isArray(data) ? data : [];
}