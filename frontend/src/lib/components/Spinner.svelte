<script lang="ts">
  export let size: 'sm' | 'md' = 'md';
</script>

<svg class="spinner"
     width={size === 'sm' ? 20 : 32}
     height={size === 'sm' ? 20 : 32}
     viewBox="0 0 32 32">
  <circle cx="16" cy="16" r="14"
          fill="none"
          stroke="var(--tcs-border)"
          stroke-width="3" />
  <circle cx="16" cy="16" r="14"
          fill="none"
          stroke="var(--tcs-primary)"
          stroke-width="3"
          stroke-dasharray="88"
          stroke-dashoffset="66"
          class="spinner-ring" />
</svg>

<style>
  .spinner-ring {
    animation: spin 0.8s linear infinite;
    transform-origin: center;
  }
  
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
