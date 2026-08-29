/** Canonical API types (camelCase — matches backend serde rename_all). */

export interface Cluster {
  id: string;
  name: string;
  controlPlaneVersion: string;
  talosVersion: string;
  status: string;
  controlPlaneSize: number;
  workerSize: number;
  hasTalosconfig?: boolean;
  hasKubeconfig?: boolean;
  backupRetention?: number | null;
  backupScheduleHours?: number | null;
  lastAutoBackupAt?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface Machine {
  id: string;
  systemUuid: string;
  machineType: string;
  clusterId: string | null;
  status: string;
  talosVersion: string;
  secureBoot: boolean;
  siderolinkConnected: boolean;
  address: string;
  installDisk?: string;
  macAddress?: string;
  hostname?: string;
  bmcAddress?: string;
  bmcUsername?: string;
  bmcType?: string;
  lastPowerState?: string;
  hasBmc?: boolean;
  createdAt: string;
  updatedAt: string;
}

export function machineHasBmc(m: Partial<Machine>): boolean {
  if (typeof m.hasBmc === 'boolean') return m.hasBmc;
  return !!(m.bmcAddress && m.bmcAddress.trim());
}

export interface MachineExtension {
  id: string;
  source: string;
  hash: string;
}

export interface MachineVersions {
  version?: string;
  installed?: string;
  upgradable?: string | null;
  [key: string]: unknown;
}

export interface ClusterBackup {
  id: string;
  clusterId: string;
  name: string;
  status: string;
  filePath?: string | null;
  sizeBytes: number;
  createdAt: string;
  updatedAt: string;
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

export function machineLabel(m: Partial<Machine> & { id?: string }): string {
  if (m.systemUuid) {
    return m.systemUuid.length > 12 ? m.systemUuid.slice(0, 12) : m.systemUuid;
  }
  return m.address || m.id || 'machine';
}

export function isControlPlane(m: Partial<Machine>): boolean {
  const t = (m.machineType || '').toLowerCase();
  return t === 'control-plane' || t === 'controlplane';
}

export function clusterNodeCount(c: Partial<Cluster>): number {
  return (c.controlPlaneSize ?? 0) + (c.workerSize ?? 0);
}

export function formatBytes(bytes: number | undefined | null): string {
  const n = bytes ?? 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
