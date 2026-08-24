<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    connect,
    getPowerState,
    power,
    setBoot,
    mountIso,
    unmountIso,
    type BmcCreds,
  } from '$lib/redfish';

  // ── Token / label from the URL (this page is token-gated, not session-gated) ──
  function queryParam(name: string): string {
    if (typeof window === 'undefined') return '';
    return new URLSearchParams(window.location.search).get(name) || '';
  }
  const token = queryParam('token');
  const label = queryParam('label') || (typeof navigator !== 'undefined' ? `browser-${navigator.language?.split('-')[0] || 'agent'}` : 'browser-agent');

  // ── Connection state ────────────────────────────────────────────────────
  type Status = 'idle' | 'connecting' | 'connected' | 'error';
  let status = $state<Status>('idle');
  let agentId = $state('');
  let statusMsg = $state('');
  let ws: WebSocket | null = null;
  let closed = $state(false);
  let backoff = 1000;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  interface LogEntry {
    id: number;
    time: string;
    op: string;
    address: string;
    ok: boolean;
    detail: string;
  }
  let log = $state<LogEntry[]>([]);
  let logSeq = 0;

  function pushLog(entry: Omit<LogEntry, 'id' | 'time'>) {
    logSeq += 1;
    log = [
      { ...entry, id: logSeq, time: new Date().toLocaleTimeString() },
      ...log,
    ].slice(0, 200);
  }

  function wsUrl(): string {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${window.location.host}/api/proxy/tunnel?token=${encodeURIComponent(token)}`;
  }

  function open() {
    if (closed) return;
    status = 'connecting';
    statusMsg = 'Dialing TCS tunnel…';
    let sock: WebSocket;
    try {
      sock = new WebSocket(wsUrl());
    } catch (e: unknown) {
      status = 'error';
      statusMsg = e instanceof Error ? e.message : 'Failed to open WebSocket';
      scheduleReconnect();
      return;
    }
    ws = sock;

    sock.onopen = () => {
      backoff = 1000;
      sock.send(JSON.stringify({ type: 'hello', caps: ['redfish'], label }));
      statusMsg = 'Hello sent, awaiting ack…';
    };

    sock.onmessage = (ev: MessageEvent) => {
      let frame: any;
      try {
        frame = JSON.parse(typeof ev.data === 'string' ? ev.data : '');
      } catch {
        return;
      }
      if (frame?.type === 'hello.ack') {
        status = 'connected';
        agentId = frame.agentId || '';
        statusMsg = 'Connected to TCS';
        pushLog({ op: 'connect', address: '—', ok: true, detail: `agent ${agentId} online` });
      } else if (frame?.type === 'bmc.op') {
        void handleOp(frame);
      }
    };

    sock.onerror = () => {
      statusMsg = 'WebSocket error';
    };

    sock.onclose = () => {
      ws = null;
      if (closed) return;
      status = 'error';
      statusMsg = 'Disconnected from TCS';
      pushLog({ op: 'disconnect', address: '—', ok: false, detail: 'tunnel closed; reconnecting' });
      scheduleReconnect();
    };
  }

  function scheduleReconnect() {
    if (closed || reconnectTimer) return;
    const delay = backoff;
    backoff = Math.min(backoff * 2, 30000);
    statusMsg = `Reconnecting in ${Math.round(delay / 1000)}s…`;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      open();
    }, delay);
  }

  // ── Op execution (mirrors oob-agent/src/main.rs execute()) ──────────────
  async function handleOp(frame: any) {
    const opId: string = frame.opId || '';
    const op: any = frame.op || {};
    const creds: BmcCreds = op.creds || {};
    const address = creds.address || '—';
    const opName = op.op || 'unknown';
    const started = Date.now();

    try {
      const ctx = await connect(creds);
      let powerState: string | undefined;
      switch (opName) {
        case 'power':
          await power(ctx, op.action || '');
          break;
        case 'set_boot':
          await setBoot(ctx, op.target === 'pxe' ? 'pxe' : 'disk', !!op.once);
          break;
        case 'get_power_state':
          powerState = await getPowerState(ctx);
          break;
        case 'mount_iso':
          await mountIso(ctx, op.isoUrl || '', op.media || '');
          break;
        case 'unmount_iso':
          await unmountIso(ctx, op.media || '');
          break;
        default:
          throw new Error(`unknown op: ${opName}`);
      }
      sendResp(opId, true, undefined, powerState);
      pushLog({ op: opName, address, ok: true, detail: `${powerState ? `state=${powerState} · ` : ''}${Date.now() - started}ms` });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      sendResp(opId, false, msg, undefined);
      pushLog({ op: opName, address, ok: false, detail: `${msg} · ${Date.now() - started}ms` });
    }
  }

  function sendResp(opId: string, ok: boolean, error: string | undefined, powerState: string | undefined) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: 'resp', opId, ok, error, powerState }));
  }

  function openBmc(address: string) {
    const host = (address || '').replace(/^https?:\/\//, '').replace(/\/+$/, '');
    if (host) window.open(`https://${host}`, '_blank', 'noopener');
  }

  onMount(() => {
    if (!token) {
      status = 'error';
      statusMsg = 'Missing ?token= query parameter. Open this page from Settings → OOB Proxy.';
      return;
    }
    open();
  });

  onDestroy(() => {
    closed = true;
    if (reconnectTimer) clearTimeout(reconnectTimer);
    if (ws) {
      ws.onclose = null;
      ws.close();
    }
  });

  const dotClass = $derived(
    status === 'connected' ? 'dot ok' : status === 'connecting' ? 'dot busy' : 'dot bad'
  );
</script>

<div class="agent">
  <header class="bar">
    <div class="title">
      <span class={dotClass}></span>
      <h1>OOB browser agent</h1>
    </div>
    <div class="meta">
      {#if agentId}<span class="chip">agent <code>{agentId}</code></span>{/if}
      <span class="chip">label <code>{label}</code></span>
      <span class="chip">caps <code>redfish</code></span>
    </div>
  </header>

  <section class="card">
    <div class="statusline">
      <span class="statuslabel">{status}</span>
      <span class="statusmsg">{statusMsg}</span>
    </div>
    {#if !token}
      <p class="err">
        No join token supplied. Create one under <a href="/settings/proxy">Settings → OOB Proxy</a> and
        open the agent from there.
      </p>
    {:else}
      <p class="hint">
        This page relays Redfish BMC operations from TCS to the BMCs reachable from <em>this</em>
        browser. Keep it open. If a BMC uses a self-signed certificate, open it once (button appears
        per failed op) and accept the warning, then the next operation will succeed.
      </p>
    {/if}
  </section>

  <section class="card">
    <h2>Operation log</h2>
    {#if log.length === 0}
      <p class="muted">Waiting for operations…</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Op</th>
            <th>BMC</th>
            <th>Result</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each log as e (e.id)}
            <tr class:fail={!e.ok}>
              <td class="mono">{e.time}</td>
              <td>{e.op}</td>
              <td class="mono">{e.address}</td>
              <td class="detail">{e.detail}</td>
              <td>
                {#if !e.ok && e.address !== '—'}
                  <button type="button" class="linkbtn" title="Open the BMC in a new tab to accept its certificate" onclick={() => openBmc(e.address)}>open BMC</button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .agent {
    min-height: 100vh;
    padding: 1.5rem;
    max-width: 60rem;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.75rem;
  }
  .title { display: flex; align-items: center; gap: 0.6rem; }
  .title h1 { margin: 0; font-size: 1.25rem; }
  .meta { display: flex; gap: 0.4rem; flex-wrap: wrap; }
  .chip {
    font-size: 0.75rem;
    color: var(--tcs-text-muted);
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 999px;
    padding: 0.2rem 0.6rem;
  }
  .chip code { color: var(--tcs-text); }
  .dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
  .dot.ok { background: #22c55e; box-shadow: 0 0 8px #22c55e; }
  .dot.busy { background: #eab308; animation: pulse 1s infinite; }
  .dot.bad { background: #ef4444; }
  @keyframes pulse { 50% { opacity: 0.4; } }
  .card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 10px;
    padding: 1rem 1.25rem;
  }
  .card h2 { margin: 0 0 0.75rem; font-size: 1rem; }
  .statusline { display: flex; align-items: baseline; gap: 0.6rem; }
  .statuslabel {
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 0.72rem;
    color: var(--tcs-text-muted);
  }
  .statusmsg { font-size: 0.9rem; }
  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; margin: 0.75rem 0 0; }
  .err { color: var(--tcs-error); font-size: 0.9rem; margin: 0.75rem 0 0; }
  .err a { color: var(--tcs-secondary); }
  .muted { color: var(--tcs-text-muted); font-size: 0.85rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th, td { text-align: left; padding: 0.4rem 0.4rem; border-bottom: 1px solid var(--tcs-border); vertical-align: top; }
  th { color: var(--tcs-text-muted); font-weight: 500; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.78rem; }
  .detail { color: var(--tcs-text-muted); word-break: break-word; }
  tr.fail td.detail { color: var(--tcs-error); }
  .linkbtn {
    background: none;
    border: none;
    color: var(--tcs-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.78rem;
    padding: 0;
  }
  .linkbtn:hover { text-decoration: underline; }
</style>
