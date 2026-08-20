<script lang="ts">
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { openSession, type TermSocket, type ContainerDetail } from '$lib/api/k8s';

  let {
    clusterId,
    ns,
    name,
    containers = [],
    containerDetails = [],
    mode = 'exec',
    command = [],
    onExit,
  }: {
    clusterId: string;
    ns: string;
    name: string;
    containers?: string[];
    containerDetails?: ContainerDetail[];
    mode?: 'exec' | 'attach';
    command?: string[];
    onExit?: (code: number) => void;
  } = $props();

  let container = $state('');
  let tty = $state(true);
  let cmdInput = $state(command.length ? command.join(' ') : 'sh');
  let termEl: HTMLDivElement | undefined = $state();
  let status = $state<'connecting' | 'running' | 'exited'>('connecting');

  // The container currently targeted (by name, else the first). Used to surface
  // its real entrypoint so distroless pods (no sh/bash) are actionable.
  const activeDetail = $derived(
    containerDetails.find((c) => c.name === container) ?? containerDetails[0] ?? null
  );
  const entrypoint = $derived(activeDetail?.command?.[0] ?? null);

  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let socket: TermSocket | undefined;

  function connect() {
    if (!termEl || !term) return;
    socket?.close();
    status = 'connecting';
    term.clear();

    // exec: parse the command input. A bare "sh"/"bash" (with TTY) is an
    // interactive shell; anything else runs that command. attach: no command.
    const cmd =
      mode === 'exec' ? cmdInput.trim().split(/\s+/).filter(Boolean) : undefined;

    socket = openSession(
      clusterId,
      mode,
      { ns, name, container: container || undefined, tty, command: cmd },
      (stream, bytes) => {
        const s = new TextDecoder().decode(bytes);
        if (stream === 'stderr') term?.write(`\x1b[31m${s}\x1b[0m`);
        else term?.write(s);
      },
      (code) => {
        status = 'exited';
        term?.write(`\r\n\x1b[90m[process exited with code ${code}]\x1b[0m`);
        onExit?.(code);
      }
    );

    term.onData((d) => socket?.sendStdin(new TextEncoder().encode(d)));
  }

  onMount(() => {
    if (!termEl) return;
    term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      theme: { background: '#0b0e14' },
      convertEol: true,
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(termEl);
    fit.fit();
    connect();

    const ro = new ResizeObserver(() => {
      fit?.fit();
      if (term) socket?.resize(term.cols, term.rows);
    });
    ro.observe(termEl);
    return () => {
      ro.disconnect();
      socket?.close();
      term?.dispose();
    };
  });
</script>

<div class="term-panel">
  <div class="term-toolbar">
    <span class="mode-badge">{mode}</span>
    {#if containers.length > 1}
      <label class="lbl">
        Container
        <select bind:value={container} onchange={connect}>
          <option value="">(default)</option>
          {#each containers as c (c)}
            <option value={c}>{c}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="chk">
      <input type="checkbox" bind:checked={tty} onchange={connect} />
      TTY
    </label>
    {#if mode === 'exec'}
      <label class="lbl cmd-lbl">
        Command
        <input
          type="text"
          bind:value={cmdInput}
          placeholder="sh  (interactive) — or any command, e.g. /coredns -version"
          onkeydown={(e) => e.key === 'Enter' && connect()}
        />
      </label>
    {/if}
    <span class="status" class:connecting={status === 'connecting'} class:exited={status === 'exited'}>
      {status}
    </span>
    <button class="btn" onclick={connect}>Connect</button>
  </div>
  {#if mode === 'exec'}
    <div class="term-hint">
      {#if activeDetail}
        <span class="hint-img" title="Container image">
          <code>{activeDetail.image}</code>
        </span>
      {/if}
      <span>
        Leave the command as <code>sh</code> for an interactive shell. If that fails —
        distroless images have no <code>sh</code>/<code>bash</code> — run a specific
        command instead, or use the Logs tab for output.
      </span>
      {#if entrypoint}
        <button
          class="fill-btn"
          title="Fill the command field with this container's entrypoint"
          onclick={() => (cmdInput = entrypoint)}
        >
          Use {entrypoint}
        </button>
      {/if}
    </div>
  {/if}
  <div class="term-host" bind:this={termEl}></div>
</div>

<style>
  .term-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: #0b0e14;
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .term-toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
    background: var(--tcs-surface);
    flex-wrap: wrap;
  }
  .mode-badge {
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    background: rgba(79, 139, 255, 0.15);
    color: var(--tcs-secondary);
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.78rem;
    color: var(--tcs-text-muted);
  }
  .lbl select {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text);
    padding: 0.25rem 0.4rem;
    font-size: 0.8rem;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.78rem;
    color: var(--tcs-text-muted);
  }
  .cmd-lbl input {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text);
    padding: 0.25rem 0.4rem;
    font-size: 0.8rem;
    font-family: ui-monospace, monospace;
    min-width: 220px;
  }
  .term-hint {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.25rem 0.5rem;
    padding: 0.4rem 0.75rem;
    font-size: 0.74rem;
    color: var(--tcs-text-muted);
    background: var(--tcs-surface);
    border-bottom: 1px solid var(--tcs-border);
  }
  .term-hint .hint-img {
    opacity: 0.85;
  }
  .term-hint .hint-img code {
    font-size: 0.72rem;
  }
  .term-hint code {
    font-family: ui-monospace, monospace;
    background: rgba(79, 139, 255, 0.12);
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }
  .term-hint .fill-btn {
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 4px;
    color: var(--tcs-secondary);
    font-size: 0.72rem;
    padding: 0.1rem 0.45rem;
    cursor: pointer;
  }
  .term-hint .fill-btn:hover {
    border-color: var(--tcs-secondary);
  }
  .status {
    font-size: 0.75rem;
    color: var(--tcs-text-muted);
    margin-left: auto;
  }
  .status.connecting {
    color: var(--tcs-warning);
  }
  .status.exited {
    color: var(--tcs-error, #ef4444);
  }
  .btn {
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text);
    font-size: 0.78rem;
    padding: 0.3rem 0.6rem;
    cursor: pointer;
  }
  .btn:hover {
    border-color: var(--tcs-text-muted);
  }
  .term-host {
    flex: 1;
    min-height: 0;
    padding: 0.25rem;
  }
  .term-host :global(.xterm) {
    height: 100%;
  }
</style>
