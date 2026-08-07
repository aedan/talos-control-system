<script lang="ts">
  import { branding } from '$lib/stores/branding';
  
  export let size: 'sm' | 'md' | 'lg' = 'md';
  
  const widthMap = { sm: 140, md: 280, lg: 420 };
</script>

<svg viewBox="0 0 280 60"
     width={widthMap[size]}
     class="logo-svg"
     xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="logoGrad" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" style="stop-color:{$branding.primaryColor};stop-opacity:1" />
      <stop offset="100%" style="stop-color:{$branding.textColor};stop-opacity:0.9" />
    </linearGradient>
  </defs>
  <text x="140" y="42"
        font-family="{$branding.fontFamily}"
        font-size="36"
        font-weight="700"
        text-anchor="middle"
        fill="url(#logoGrad)">{$branding.shortName}</text>
</svg>

<style>
  .logo-svg {
    display: block;
  }
</style>
