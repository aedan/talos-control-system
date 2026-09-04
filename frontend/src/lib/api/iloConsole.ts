// Client for the machine OOB console endpoints.
//
// Two modes:
//   - `mode: 'ilo'` — the server minted an iLO HTML5 session. Load `embed_url`
//     in an <iframe>; the server reverse-proxies the iLO console assets and
//     relays the KVM WebSocket, all same-origin (defeats X-Frame-Options).
//   - `mode: 'sol'` — Serial-over-LAN. Open the SOL WebSocket (xterm.js). An
//     optional `idrac_console_url` offers Dell's native console in a new tab.
//
// SOL / KVM WebSockets authenticate via `?token=` because WS clients cannot
// set an Authorization header.

import { client } from './client';

const API_BASE = '/api';

export interface ConsoleSession {
  ok: boolean;
  mode: 'ilo' | 'sol' | 'none';
  session_id: string | null;
  embed_url: string | null;
  idrac_console_url: string | null;
  shared: boolean | null;
  viewers: number | null;
  error: string | null;
}

function token(): string | null {
  if (typeof window === 'undefined') return null;
  return localStorage.getItem('tcs_token');
}

/** Mint (or reuse) a console session for a machine. Never throws. */
export async function openConsoleSession(
  machineId: string,
  force = false,
): Promise<ConsoleSession> {
  const qs = force ? '?force=true' : '';
  const res = await client.post(`/machines/${machineId}/console/session${qs}`);
  return res as ConsoleSession;
}

/** Best-effort logout of an iLO session (called on overlay close). */
export async function closeConsoleSession(
  machineId: string,
  sessionId: string,
): Promise<void> {
  try {
    await client.post(
      `/machines/${machineId}/console/session/close?sid=${encodeURIComponent(sessionId)}`,
    );
  } catch {
    // Non-fatal: the session expires server-side after its TTL.
  }
}

export interface SolHandle {
  close: () => void;
  send: (data: string) => void;
  ready: () => boolean;
}

/**
 * Open the Dell SOL WebSocket for a machine.
 *
 * `onData(bytes)` receives raw stdout/stderr (text). `send(str)` writes
 * stdin. SOL is byte-transparent — feed xterm.js output straight in.
 */
export function openSolSession(
  machineId: string,
  onData: (bytes: Uint8Array) => void,
  onExit: (reason: string) => void,
): SolHandle {
  const t = token();
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
  const url = `${proto}://${window.location.host}${API_BASE}/machines/${machineId}/console/sol?token=${encodeURIComponent(t ?? '')}`;
  const ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';

  ws.onopen = () => {
    // Nothing to prime; SOL is interactive.
  };

  ws.onmessage = (ev) => {
    if (typeof ev.data === 'string') {
      onData(new TextEncoder().encode(ev.data));
    } else if (ev.data instanceof ArrayBuffer) {
      onData(new Uint8Array(ev.data));
    } else if (ev.data instanceof Blob) {
      ev.data.arrayBuffer().then((ab) => onData(new Uint8Array(ab)));
    }
  };

  ws.onerror = () => {
    onExit('connection error');
  };

  ws.onclose = () => {
    onExit('closed');
  };

  return {
    close: () => {
      try {
        ws.close();
      } catch {
        // ignore
      }
    },
    send: (data: string) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
      }
    },
    ready: () => ws.readyState === WebSocket.OPEN,
  };
}
