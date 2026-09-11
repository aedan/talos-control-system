import { client } from '$lib/api/client';

/**
 * Types + client for the "Convert to Talos" in-place conversion feature.
 * Backend contract is being built in parallel; everything is camelCase.
 */

export interface ConvertNetworkInterface {
  name: string;
  mtu: number;
  ip: string;
  cidr: string;
  mac: string;
}

export interface ConvertNetworkBond {
  name: string;
  mode: string;
  slaves: string[];
}

export interface ConvertNetworkVlan {
  name: string;
  id: number;
  parent: string;
}

export interface ConvertNetwork {
  interfaces: ConvertNetworkInterface[];
  bonds: ConvertNetworkBond[];
  vlans: ConvertNetworkVlan[];
  gateway: string;
  dns: string[];
  ovsBridges: string[];
}

export interface ConvertNode {
  name: string;
  role: 'control-plane' | 'worker';
  osType: string; // "talos" | "baremetal"
  osImage: string;
  sshOk: boolean;
  sshError: string; // "" if ok
  drivers: string[];
  recommendedModules: string[];
  network: ConvertNetwork;
}

export interface ConvertEtcd {
  cpNodes: string[];
  embedded: boolean;
}

export interface ConvertPreview {
  clusterName: string;
  talosVersion: string;
  kubernetesVersion: string;
  nodes: ConvertNode[];
  etcd: ConvertEtcd;
  canConvert: boolean;
  blockers: string[];
}

export interface ConvertJobNode {
  name: string;
  role: string;
  status: string; // pending|cordon|drain|kexec|install|reboot|join|recover|done|failed|skipped
  currentStep: string;
  error: string;
}

export interface ConvertEtcdSnapshot {
  taken: boolean;
  sizeBytes: number;
}

export interface ConvertStatus {
  jobId: string;
  status: 'running' | 'complete' | 'failed' | 'cancelled';
  phase: 'snapshot' | 'control-plane' | 'workers' | 'adopt' | 'done';
  etcdSnapshot: ConvertEtcdSnapshot;
  nodes: ConvertJobNode[];
  stepsLog: string[];
}

export interface ConvertStartResult {
  jobId: string;
  status: string;
}

export interface FactoryExtensionItem {
  name: string;
  ref: string | null;
  description: string | null;
  author: string | null;
}

export async function convertPreview(
  id: string,
  body: { talosVersion: string; modules: string[] }
): Promise<ConvertPreview> {
  const res = await client.post(`/clusters/${id}/convert/preview`, body);
  return res as ConvertPreview;
}

export async function convertStart(
  id: string,
  body: { talosVersion: string; modules: string[]; nodes: { name: string; role: string }[] }
): Promise<ConvertStartResult> {
  const res = await client.post(`/clusters/${id}/convert/start`, body);
  return res as ConvertStartResult;
}

export async function convertStatus(id: string): Promise<ConvertStatus> {
  const res = await client.get(`/clusters/${id}/convert`);
  return res as ConvertStatus;
}

export async function convertCancel(id: string): Promise<{ ok: boolean }> {
  const res = await client.post(`/clusters/${id}/convert/cancel`, {});
  return res as { ok: boolean };
}

export async function fetchFactoryExtensions(version: string): Promise<FactoryExtensionItem[]> {
  const res = (await client.get(
    `/factory/extensions?version=${encodeURIComponent(version)}`
  )) as { extensions: FactoryExtensionItem[] };
  return res.extensions || [];
}
