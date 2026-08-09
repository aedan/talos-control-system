import { writable } from 'svelte/store';
import { client } from '$lib/api/client';

export interface MachineClass {
  id: string;
  name: string;
  description: string;
  minCpu: number;
  minMemory: number;
  minDisk: number;
  arch: string;
  secureBoot: boolean;
  allowedRoles: string[];
  createdAt: string;
  updatedAt: string;
}

export interface CreateMachineClassPayload {
  name: string;
  description: string;
  minCpu: number;
  minMemory: number;
  minDisk: number;
  arch: string;
  secureBoot: boolean;
  allowedRoles: string[];
}

export const machineClasses = writable<MachineClass[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

export async function loadMachineClasses(): Promise<void> {
  loading.set(true);
  error.set(null);
  try {
    const data = await client.get('/machine-classes') as MachineClass[];
    machineClasses.set(data);
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to load machine classes');
  } finally {
    loading.set(false);
  }
}

export async function createMachineClass(payload: CreateMachineClassPayload): Promise<MachineClass> {
  error.set(null);
  try {
    const data = await client.post('/machine-classes', payload);
    const mc = data as MachineClass;
    machineClasses.update((items) => [mc, ...items]);
    return mc;
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to create machine class');
    throw e;
  }
}

export async function updateMachineClass(id: string, payload: CreateMachineClassPayload): Promise<MachineClass> {
  error.set(null);
  try {
    const data = await client.put(`/machine-classes/${id}`, payload);
    const mc = data as MachineClass;
    machineClasses.update((items) =>
      items.map((item) => (item.id === id ? mc : item))
    );
    return mc;
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to update machine class');
    throw e;
  }
}

export async function deleteMachineClass(id: string): Promise<void> {
  error.set(null);
  try {
    await client.delete(`/machine-classes/${id}`);
    machineClasses.update((items) => items.filter((mc) => mc.id !== id));
  } catch (e: unknown) {
    error.set(e instanceof Error ? e.message : 'Failed to delete machine class');
    throw e;
  }
}
