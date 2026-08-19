<script lang="ts">
  import YAML from 'yaml';

  let {
    value,
    title = 'YAML',
  }: {
    value: unknown;
    title?: string;
  } = $props();

  // Accept a raw object, a JSON string, or a YAML string.
  let text = $derived.by(() => {
    if (value == null) return '';
    if (typeof value === 'string') {
      // If it already looks like YAML/JSON text, show as-is when it parses as YAML.
      try {
        return YAML.stringify(YAML.parse(value));
      } catch {
        return value;
      }
    }
    try {
      return YAML.stringify(value);
    } catch {
      return JSON.stringify(value, null, 2);
    }
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* ignore */
    }
  }
</script>

<div class="yaml-view">
  <div class="yaml-head">
    <span class="yaml-title">{title}</span>
    <button class="copy-btn" onclick={copy}>Copy</button>
  </div>
  <pre class="yaml-pre"><code>{text}</code></pre>
</div>

<style>
  .yaml-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .yaml-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--tcs-border);
    background: var(--tcs-surface);
  }
  .yaml-title {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tcs-text-muted);
    font-weight: 600;
  }
  .copy-btn {
    background: none;
    border: 1px solid var(--tcs-border);
    border-radius: 4px;
    color: var(--tcs-text-muted);
    font-size: 0.72rem;
    padding: 0.2rem 0.5rem;
    cursor: pointer;
  }
  .copy-btn:hover {
    color: var(--tcs-text);
    border-color: var(--tcs-text-muted);
  }
  .yaml-pre {
    margin: 0;
    padding: 0.75rem;
    overflow: auto;
    flex: 1;
    min-height: 0;
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    line-height: 1.5;
    color: var(--tcs-text);
    white-space: pre;
  }
</style>
