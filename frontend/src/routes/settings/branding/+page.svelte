<script lang="ts">
  import { branding, applyBranding } from '$lib/stores/branding';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Logo from '$lib/branding/components/Logo.svelte';
  
  let form = $state({ ...$branding });
  let saving = $state(false);
  
  async function save() {
    saving = true;
    try {
      await client.put('/branding', {
        name: form.name,
        shortName: form.shortName,
        tagline: form.tagline,
        primaryColor: form.primaryColor,
        secondaryColor: form.secondaryColor,
        backgroundColor: form.backgroundColor,
        surfaceColor: form.surfaceColor,
        textColor: form.textColor,
        textMutedColor: form.textMutedColor,
        fontFamily: form.fontFamily,
        docsUrl: form.docsUrl,
        supportUrl: form.supportUrl
      });
      branding.set(form);
      applyBranding(form);
      success('Branding updated successfully');
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to update branding');
    } finally {
      saving = false;
    }
  }
  
  function reset() {
    form = {
      name: 'Talos Control System',
      shortName: 'TCS',
      tagline: 'Kubernetes Management Simplified',
      primaryColor: '#150D6A',
      secondaryColor: '#4F8BFF',
      backgroundColor: '#0A0A0A',
      surfaceColor: '#1A1A1A',
      textColor: '#FFFFFF',
      textMutedColor: '#A0A0A0',
      fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
      logoPath: '/branding/logo.svg',
      faviconPath: '/branding/favicon.svg',
      docsUrl: '',
      supportUrl: ''
    };
  }
</script>

<div class="branding-page">
  <h1>White-Label Branding</h1>
  <p class="description">Customize the appearance and identity of your TCS deployment.</p>
  
  <div class="branding-grid">
    <div class="branding-form">
      <div class="section">
        <h2>Identity</h2>
        <div class="form-group">
          <label for="name">Platform Name</label>
          <input id="name" type="text" bind:value={form.name} />
        </div>
        <div class="form-group">
          <label for="shortName">Short Name</label>
          <input id="shortName" type="text" bind:value={form.shortName} />
        </div>
        <div class="form-group">
          <label for="tagline">Tagline</label>
          <input id="tagline" type="text" bind:value={form.tagline} />
        </div>
      </div>
      
      <div class="section">
        <h2>Colors</h2>
        <div class="color-row">
          <div class="form-group">
            <label for="primary">Primary</label>
            <div class="color-input">
              <input id="primary" type="color" bind:value={form.primaryColor} />
              <input type="text" bind:value={form.primaryColor} />
            </div>
          </div>
          <div class="form-group">
            <label for="secondary">Secondary</label>
            <div class="color-input">
              <input id="secondary" type="color" bind:value={form.secondaryColor} />
              <input type="text" bind:value={form.secondaryColor} />
            </div>
          </div>
          <div class="form-group">
            <label for="bg">Background</label>
            <div class="color-input">
              <input id="bg" type="color" bind:value={form.backgroundColor} />
              <input type="text" bind:value={form.backgroundColor} />
            </div>
          </div>
          <div class="form-group">
            <label for="surface">Surface</label>
            <div class="color-input">
              <input id="surface" type="color" bind:value={form.surfaceColor} />
              <input type="text" bind:value={form.surfaceColor} />
            </div>
          </div>
        </div>
      </div>
      
      <div class="section">
        <h2>Typography</h2>
        <div class="form-group">
          <label for="font">Font Family</label>
          <input id="font" type="text" bind:value={form.fontFamily} />
        </div>
      </div>
      
      <div class="section">
        <h2>Links</h2>
        <div class="form-group">
          <label for="docs">Documentation URL</label>
          <input id="docs" type="url" bind:value={form.docsUrl} placeholder="Leave empty to hide" />
        </div>
        <div class="form-group">
          <label for="support">Support URL</label>
          <input id="support" type="url" bind:value={form.supportUrl} placeholder="Leave empty to hide" />
        </div>
      </div>
    </div>
    
    <div class="branding-preview">
      <h2>Preview</h2>
      <div class="preview-card">
        <Logo size="lg" />
        <p class="preview-tagline">{form.tagline}</p>
      </div>
      
      <div class="preview-bar">
        <span style="color: {form.textColor}">Text</span>
        <span style="color: {form.textMutedColor}">Muted</span>
        <span style="color: {form.secondaryColor}">Secondary</span>
      </div>
      
      <div class="actions">
        <Button variant="primary" onclick={save} disabled={saving}>
          {saving ? 'Saving...' : 'Save Changes'}
        </Button>
        <Button variant="ghost" onclick={reset}>Reset to Defaults</Button>
      </div>
    </div>
  </div>
</div>

<style>
  .branding-page h1 { margin: 0 0 0.5rem; }
  .description { color: var(--tcs-text-muted); margin-bottom: 2rem; }
  
  .branding-grid { display: grid; grid-template-columns: 1fr 340px; gap: 2rem; }
  @media (max-width: 900px) {
    .branding-grid { grid-template-columns: 1fr; }
  }
  
  .section { margin-bottom: 2rem; }
  .section h2 { font-size: 1rem; margin: 0 0 1rem; color: var(--tcs-text-muted); }
  .form-group { display: flex; flex-direction: column; gap: 0.4rem; margin-bottom: 1rem; }
  .form-group label { color: var(--tcs-text-muted); font-size: 0.875rem; }
  .form-group input {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    outline: none;
  }
  .form-group input:focus { border-color: var(--tcs-primary); }
  
  .color-row { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
  .color-input { display: flex; gap: 0.5rem; }
  .color-input input[type="color"] {
    width: 40px; height: 40px; padding: 2px; cursor: pointer;
  }
  .color-input input[type="text"] { flex: 1; }
  
  .preview-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 2rem;
    text-align: center;
    margin-bottom: 1rem;
  }
  .preview-tagline { color: var(--tcs-text-muted); font-size: 0.875rem; }
  
  .preview-bar {
    display: flex; justify-content: space-between;
    padding: 0.75rem;
    background: var(--tcs-surface);
    border-radius: 6px;
    font-size: 0.8rem;
    margin-bottom: 1rem;
  }
  
  .actions { display: flex; flex-direction: column; gap: 0.5rem; }
</style>
