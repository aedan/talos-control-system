// Typed client for the K8s explorer + CLI proxy endpoints.
//
// All calls go through the TCS server, which holds the kubeconfig. The browser
// never sees a kubeconfig. Reads/logs/exec inherit RBAC; mutations require admin.
//
// SSE and WebSocket endpoints authenticate via `?token=` because those clients
// cannot set an Authorization header.

import { client } from './client';

const API_BASE = '/api';

// ---- Summary types (mirror backend/src/integration/k8s_explorer.rs) ----

export interface NamespaceSummary {
  name: string;
  status: string;
  age: string;
}

export interface PodSummary {
  name: string;
  namespace: string;
  phase: string;
  ready: string;
  restarts: number;
  node: string;
  ip: string;
  age: string;
  containers: string[];
}

export interface DeploymentSummary {
  name: string;
  namespace: string;
  ready: string;
  replicas: number;
  available: number;
  age: string;
}

export interface ServiceSummary {
  name: string;
  namespace: string;
  kind: string;
  clusterIp: string;
  ports: string;
  age: string;
}

export interface EventSummary {
  namespace: string;
  name: string;
  kind: string;
  reason: string;
  message: string;
  object: string;
  count: number;
  lastSeen: string;
}

export interface NodeSummary {
  name: string;
  status: string;
  role: string;
  kubernetesVersion: string;
  osImage: string;
  internalIp: string;
  age: string;
}

export interface ContainerDetail {
  name: string;
  image: string;
  ready: boolean;
  restarts: number;
  state: string;
  startedAt: string;
  lastState: string;
  /** Container spec `command` (overrides the image entrypoint). Empty = image entrypoint runs. */
  command: string[];
  /** Container spec `args`. */
  args: string[];
}

export interface PodDetail {
  summary: PodSummary;
  containers: ContainerDetail[];
  labels: Record<string, string>;
  yaml: string;
}

export interface ResolvedKind {
  group: string;
  version: string;
  apiVersion: string;
  kind: string;
  plural: string;
  namespaced: boolean;
}

export interface ApplyResult {
  kind: string;
  name: string;
  namespace: string;
  status: string;
}

export interface ApplyResponse {
  ok: boolean;
  results: ApplyResult[];
}

export interface DrainResult {
  node: string;
  evicted: string[];
  skipped: string[];
  errors: string[];
}

export interface ScaleResponse {
  ok: boolean;
  name: string;
  replicas: number;
}

export interface NodeResponse {
  ok: boolean;
  node: string;
  cordoned: boolean;
}

export interface DeleteResponse {
  ok: boolean;
  deleted: string;
}

// ---- Query option helpers ----

function qs(params: Record<string, string | number | boolean | undefined>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '') continue;
    parts.push(`${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`);
  }
  return parts.length ? `?${parts.join('&')}` : '';
}

function token(): string | null {
  if (typeof window === 'undefined') return null;
  return localStorage.getItem('tcs_token');
}

// ---- Read endpoints ----

export async function listKinds(clusterId: string): Promise<ResolvedKind[]> {
  return client.get(`/clusters/${clusterId}/k8s/kinds`) as Promise<ResolvedKind[]>;
}

export async function listNamespaces(clusterId: string): Promise<NamespaceSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/namespaces`) as Promise<NamespaceSummary[]>;
}

export async function listPods(clusterId: string, ns?: string): Promise<PodSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/pods${qs({ ns })}`) as Promise<PodSummary[]>;
}

export async function getPod(clusterId: string, ns: string, name: string): Promise<PodDetail> {
  return client.get(`/clusters/${clusterId}/k8s/pods/${ns}/${name}`) as Promise<PodDetail>;
}

export async function listDeployments(clusterId: string, ns?: string): Promise<DeploymentSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/deployments${qs({ ns })}`) as Promise<DeploymentSummary[]>;
}

export async function listServices(clusterId: string, ns?: string): Promise<ServiceSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/services${qs({ ns })}`) as Promise<ServiceSummary[]>;
}

export async function listEvents(clusterId: string, ns?: string): Promise<EventSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/events${qs({ ns })}`) as Promise<EventSummary[]>;
}

export async function listNodes(clusterId: string): Promise<NodeSummary[]> {
  return client.get(`/clusters/${clusterId}/k8s/nodes`) as Promise<NodeSummary[]>;
}

/** List an arbitrary kind (returns raw K8s JSON list object). */
export async function listResource(clusterId: string, kind: string, ns?: string): Promise<unknown> {
  return client.get(`/clusters/${clusterId}/k8s/resource${qs({ kind, ns })}`);
}

/** Get one object of an arbitrary kind (returns raw K8s JSON). */
export async function getResource(clusterId: string, kind: string, name: string, ns?: string): Promise<unknown> {
  return client.get(`/clusters/${clusterId}/k8s/resource/${name}${qs({ kind, ns })}`);
}

// ---- Mutation endpoints (admin) ----

export async function deleteResource(clusterId: string, kind: string, name: string, ns?: string): Promise<DeleteResponse> {
  return client.delete(`/clusters/${clusterId}/k8s/resource/${name}${qs({ kind, ns })}`) as Promise<DeleteResponse>;
}

export async function scaleDeployment(clusterId: string, ns: string, name: string, replicas: number): Promise<ScaleResponse> {
  return client.post(`/clusters/${clusterId}/k8s/scale`, { ns, name, replicas }) as Promise<ScaleResponse>;
}

export async function cordonNode(clusterId: string, name: string): Promise<NodeResponse> {
  return client.post(`/clusters/${clusterId}/k8s/cordon`, { name }) as Promise<NodeResponse>;
}

export async function uncordonNode(clusterId: string, name: string): Promise<NodeResponse> {
  return client.post(`/clusters/${clusterId}/k8s/uncordon`, { name }) as Promise<NodeResponse>;
}

export async function drainNode(clusterId: string, name: string, force = false): Promise<DrainResult> {
  return client.post(`/clusters/${clusterId}/k8s/drain`, { name, force }) as Promise<DrainResult>;
}

export async function applyManifest(clusterId: string, manifest: string): Promise<ApplyResponse> {
  return client.post(`/clusters/${clusterId}/k8s/apply`, { manifest }) as Promise<ApplyResponse>;
}

// ---- Streaming endpoints ----

export interface LogsOptions {
  ns: string;
  name: string;
  container?: string;
  tail?: number;
  previous?: boolean;
  follow?: boolean;
}

/**
 * Fetch non-following logs as text. For `follow`, use `streamLogs` (SSE).
 */
export async function getLogs(clusterId: string, opts: LogsOptions): Promise<string> {
  const o = { ...opts, follow: false };
  return client.get(`/clusters/${clusterId}/k8s/logs${qs(o)}`) as Promise<string>;
}

/**
 * Stream following logs over SSE. Returns a close function.
 * Each line is delivered to `onLine`.
 */
export function streamLogs(clusterId: string, opts: LogsOptions, onLine: (line: string) => void): () => void {
  const o = { ...opts, follow: true, token: token() ?? undefined };
  const url = `${API_BASE}/clusters/${clusterId}/k8s/logs${qs(o)}`;
  const evt = new EventSource(url);
  evt.onmessage = (e) => {
    if (e.data) onLine(e.data);
  };
  return () => evt.close();
}

export interface ExecOptions {
  ns: string;
  name: string;
  container?: string;
  tty?: boolean;
  command?: string[];
}

export interface TermSocket {
  sendStdin(data: Uint8Array): void;
  resize(cols: number, rows: number): void;
  close(): void;
  onExit(cb: (code: number) => void): void;
}

/**
 * Open an exec/attach WebSocket session.
 *
 * `onData(stream, bytes)` receives decoded stdout/stderr. `onExit` fires when the
 * process ends. Returns a handle to send stdin / resize / close.
 */
export function openSession(
  clusterId: string,
  kind: 'exec' | 'attach',
  opts: ExecOptions,
  onData: (stream: 'stdout' | 'stderr', bytes: Uint8Array) => void,
  onExit: (code: number) => void,
): TermSocket {
  const q = qs({
    ns: opts.ns,
    name: opts.name,
    container: opts.container,
    tty: opts.tty,
    // JSON array preserves quoting (e.g. sh -c "a b"); the server parses it
    // and falls back to space-splitting for legacy string commands.
    command: opts.command?.length ? JSON.stringify(opts.command) : undefined,
    token: token() ?? undefined,
  });
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
  const url = `${proto}://${window.location.host}${API_BASE}/clusters/${clusterId}/k8s/${kind}${q}`;
  const ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';

  const b64 = (s: string) => btoa(s);
  const fromB64 = (s: string) => {
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  };

  ws.onmessage = (ev) => {
    if (typeof ev.data !== 'string') return;
    let msg: { type: string; data?: string; code?: number };
    try {
      msg = JSON.parse(ev.data);
    } catch {
      return;
    }
    if (msg.type === 'stdout' && msg.data) onData('stdout', fromB64(msg.data));
    else if (msg.type === 'stderr' && msg.data) onData('stderr', fromB64(msg.data));
    else if (msg.type === 'exit') onExit(msg.code ?? 0);
  };

  return {
    sendStdin(data) {
      if (ws.readyState === WebSocket.OPEN) {
        // btoa on the binary string of the bytes.
        let bin = '';
        for (let i = 0; i < data.length; i++) bin += String.fromCharCode(data[i]);
        ws.send(JSON.stringify({ type: 'stdin', data: b64(bin) }));
      }
    },
    resize(cols, rows) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    },
    close() {
      ws.close();
    },
    onExit(cb) {
      ws.onclose = () => cb(0);
    },
  };
}
