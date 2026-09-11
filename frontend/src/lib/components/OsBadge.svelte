<script lang="ts">
  let {
    osImage,
    isTalos,
    class: extraClass = '',
    title
  }: {
    osImage: string;
    isTalos: boolean;
    class?: string;
    title?: string;
  } = $props();

  function shortenOs(osImage: string): string {
    let s = (osImage ?? '').trim();
    if (!s) return 'Unknown OS';
    const lower = s.toLowerCase();
    const talosIdx = lower.indexOf('talos');
    if (talosIdx >= 0) s = s.slice(0, talosIdx).trim();
    const dashIdx = s.indexOf('-');
    if (dashIdx > 0) s = s.slice(0, dashIdx).trim();
    const linuxIdx = s.search(/\sLinux\s/i);
    if (linuxIdx >= 0) s = s.slice(0, linuxIdx + 6).trim();
    const parenIdx = s.indexOf('(');
    if (parenIdx >= 0) s = s.slice(0, parenIdx).trim();
    if (s.length > 40) s = s.slice(0, 37) + '...';
    return s || 'Unknown OS';
  }

  let label = $derived(isTalos ? 'Talos Linux' : shortenOs(osImage));
</script>

<span class="os-badge {isTalos ? 'talos' : 'non-talos'} {extraClass}" {title}>
  {label}
</span>

<style>
  .os-badge {
    display: inline-block;
    font-size: 0.75rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    white-space: nowrap;
  }
  .os-badge.talos { background: rgba(16, 185, 129, 0.2); color: var(--tcs-success); }
  .os-badge.non-talos { background: rgba(245, 158, 11, 0.2); color: var(--tcs-warning); }
</style>
