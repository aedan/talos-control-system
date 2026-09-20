<script lang="ts">
  import type { FactoryExtension } from '$lib/api/types';

  let {
    extensions = [],
    selected,
    busy = false,
    error = '',
    version = '',
    showPresets = true,
    onchange,
  }: {
    extensions: FactoryExtension[];
    selected: Set<string>;
    busy?: boolean;
    error?: string;
    version?: string;
    showPresets?: boolean;
    onchange: (next: Set<string>) => void;
  } = $props();

  const PRESETS: { id: string; label: string; hint: string; modules: string[] }[] = [
    {
      id: '10g-bnx2',
      label: '10Gb Broadcom NICs',
      hint: 'bnx2 / bnx2x firmware — typical 10Gb bonds',
      modules: ['siderolabs/bnx2-bnx2x'],
    },
    {
      id: 'genestack-storage',
      label: 'Genestack storage',
      hint: 'Longhorn + Ceph need iSCSI and NFS in the image',
      modules: ['siderolabs/iscsi-tools', 'siderolabs/nfs-utils'],
    },
  ];

  function shortName(full: string): string {
    const i = full.indexOf('/');
    return i >= 0 ? full.slice(i + 1) : full;
  }

  function toggle(name: string) {
    const next = new Set(selected);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onchange(next);
  }

  function presetOn(id: string): boolean {
    const p = PRESETS.find((x) => x.id === id);
    return !!p && p.modules.every((m) => selected.has(m));
  }

  function togglePreset(id: string) {
    const p = PRESETS.find((x) => x.id === id);
    if (!p) return;
    const on = !presetOn(id);
    const next = new Set(selected);
    for (const m of p.modules) {
      if (on) next.add(m);
      else next.delete(m);
    }
    onchange(next);
  }
</script>

<div class="factory-modules">
  <p class="hint">
    Optional. Bake Talos system extensions into the installer <em>and</em> the on-disk image
    (e.g. <code>siderolabs/bnx2-bnx2x</code> for Broadcom 10G NICs). Leave empty for the default
    image.
  </p>
  {#if showPresets}
    <div class="preset-row">
      {#each PRESETS as p (p.id)}
        <button type="button" class="preset-btn" class:on={presetOn(p.id)} title={p.hint} onclick={() => togglePreset(p.id)}>
          <span class="preset-label">{p.label}</span>
          <span class="preset-hint">{p.hint}</span>
        </button>
      {/each}
    </div>
  {/if}
  {#if error}
    <p class="hint error">{error}</p>
  {:else if busy}
    <p class="hint">Loading module catalog…</p>
  {:else}
    <div class="module-picker">
      {#each extensions as f (f.name)}
        <label class="module-option" title={f.description || f.ref || ''}>
          <input type="checkbox" checked={selected.has(f.name)} onchange={() => toggle(f.name)} />
          <span class="mono">{shortName(f.name)}</span>
          {#if f.author}<span class="hint-inline"> · {f.author}</span>{/if}
        </label>
      {/each}
      {#if extensions.length === 0}
        <p class="hint">No modules returned{version ? ` for ${version}` : ''}.</p>
      {/if}
    </div>
    {#if selected.size > 0}
      <p class="hint">
        Selected:
        {#each [...selected].sort() as m (m)}
          <span class="module-chip mono">{shortName(m)}</span>
        {/each}
      </p>
    {/if}
  {/if}
</div>

<style>
  .hint { color: var(--tcs-text-muted); font-size: 0.85rem; margin: 0 0 0.75rem; }
  .hint.error { color: var(--tcs-error, #ef4444); }
  .hint-inline { color: var(--tcs-text-muted); font-size: 0.8rem; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  code { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .preset-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 0.5rem;
    margin: 0 0 0.85rem;
  }
  .preset-btn {
    text-align: left;
    padding: 0.55rem 0.7rem;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    background: var(--tcs-background, var(--tcs-surface));
    color: var(--tcs-text);
    cursor: pointer;
  }
  .preset-btn.on {
    border-color: var(--tcs-primary, var(--tcs-accent, #3b82f6));
    background: color-mix(in srgb, var(--tcs-primary, #3b82f6) 12%, transparent);
  }
  .preset-label { display: block; font-size: 0.85rem; font-weight: 600; }
  .preset-hint { display: block; font-size: 0.75rem; color: var(--tcs-text-muted); margin-top: 0.15rem; line-height: 1.35; }
  .module-picker {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 0.25rem 0.75rem;
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.65rem;
    margin: 0.4rem 0;
  }
  .module-option {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    font-size: 0.82rem;
    padding: 0.1rem 0;
  }
  .module-option input { flex: 0 0 auto; }
  .module-chip {
    display: inline-block;
    background: color-mix(in srgb, var(--tcs-primary, #3b82f6) 15%, transparent);
    border: 1px solid var(--tcs-primary, #3b82f6);
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.75rem;
    margin: 0.1rem 0.2rem 0.1rem 0;
  }
</style>
