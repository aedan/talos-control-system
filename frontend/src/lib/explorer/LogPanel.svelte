<script lang="ts">
  import { getLogs, streamLogs } from '$lib/api/k8s';

  let {
    clusterId,
    ns,
    name,
    containers = [],
    follow = false,
    tail = 200,
    previous = false,
    onFollowChange,
  }: {
    clusterId: string;
    ns: string;
    name: string;
    containers?: string[];
    follow?: boolean;
    tail?: number;
    previous?: boolean;
    onFollowChange?: (f: boolean) => void;
  } = $props();

  let container = $state('');
  let text = $state('');
  let loading = $state(false);
  let error = $state('');
  let closeStream: (() => void) | null = null;
  let preEl: HTMLPreElement | undefined = $state();

  const MAX = 20000;

  function scrollDown() {
    if (preEl) preEl.scrollTop = preEl.scrollHeight;
  }

  async function loadOnce() {
    closeStream?.();
    closeStream = null;
    loading = true;
    error = '';
    text = '';
    try {
      const t = (await getLogs(clusterId, { ns, name, container: container || undefined, tail, previous, follow: false })) as string;
      text = t;
      requestAnimationFrame(scrollDown);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load logs';
    } finally {
      loading = false;
    }
  }

  function startFollow() {
    closeStream?.();
    error = '';
    text = '';
    closeStream = streamLogs(
      clusterId,
      { ns, name, container: container || undefined, tail, previous, follow: true },
      (line) => {
        text += line + '\n';
        if (text.length > MAX) text = text.slice(text.length - MAX);
        requestAnimationFrame(scrollDown);
      }
    );
  }

  function setFollow(f: boolean) {
    onFollowChange?.(f);
    follow = f;
  }

  // Single effect drives loading: re-runs when pod/container/follow/options change.
  $effect(() => {
    void ns;
    void name;
    void container;
    void tail;
    void previous;
    if (follow) startFollow();
    else loadOnce();
  });

  $effect(() => {
    return () => {
      closeStream?.();
      closeStream = null;
    };
  });
</script>

<div class="log-panel">
  <div class="log-toolbar">
    {#if containers.length > 1}
      <label class="lbl">
        Container
        <select title="Container to show logs for" bind:value={container}>
          <option value="">(default)</option>
          {#each containers as c (c)}
            <option value={c}>{c}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="lbl">
      Tail
      <input type="number" min="1" title="Number of recent log lines to fetch" bind:value={tail} />
    </label>
    <label class="chk">
      <input type="checkbox" title="Show logs from the previous container instance" bind:checked={previous} />
      Previous
    </label>
    <button class="btn" class:active={follow} title="Stream logs live / stop following" onclick={() => setFollow(!follow)}>
      {follow ? 'Stop following' : 'Follow'}
    </button>
    <button class="btn" title="Reload logs once" onclick={loadOnce}>Refresh</button>
    <button class="btn" title="Clear the log output" onclick={() => (text = '')}>Clear</button>
  </div>

  {#if error}
    <div class="log-error">{error}</div>
  {/if}
  <pre class="log-pre" bind:this={preEl}>{text}{#if loading && !text}<span class="dim">loading…</span>{/if}</pre>
</div>

<style>
  .log-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: #0b0e14;
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .log-toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
    background: var(--tcs-surface);
    flex-wrap: wrap;
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.78rem;
    color: var(--tcs-text-muted);
  }
  .lbl select,
  .lbl input {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 5px;
    color: var(--tcs-text);
    padding: 0.25rem 0.4rem;
    font-size: 0.8rem;
  }
  .lbl input {
    width: 4.5rem;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.78rem;
    color: var(--tcs-text-muted);
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
  .btn.active {
    background: rgba(79, 139, 255, 0.18);
    border-color: var(--tcs-secondary);
    color: var(--tcs-secondary);
  }
  .log-error {
    padding: 0.5rem 0.75rem;
    color: var(--tcs-error, #ef4444);
    font-size: 0.8rem;
    background: rgba(239, 68, 68, 0.08);
  }
  .log-pre {
    margin: 0;
    padding: 0.75rem;
    overflow: auto;
    flex: 1;
    min-height: 0;
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    line-height: 1.45;
    color: #d7e0ea;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .dim {
    color: #5b6675;
  }
</style>
