<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface CertStatus {
    issuer: string;
    domains: string[];
    expiryDate: string;
    daysRemaining: number;
    mode: 'letsencrypt' | 'self-signed' | 'provided' | 'disabled';
  }

  interface CertConfig {
    mode: 'letsencrypt' | 'self-signed' | 'provided' | 'disabled';
    domains: string[];
    adminEmail?: string;
    challengeType?: 'http-01' | 'dns-01';
    dnsProvider?: 'godaddy' | 'cloudflare' | 'route53';
    dnsApiKey?: string;
    dnsApiSecret?: string;
    dnsApiToken?: string;
    dnsZoneId?: string;
    certPath?: string;
    keyPath?: string;
  }

  let status = $state<CertStatus | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let renewing = $state(false);
  let error = $state('');
  let successMsg = $state('');

  let config = $state<CertConfig>({
    mode: 'disabled',
    domains: [],
    adminEmail: '',
    challengeType: 'http-01',
    dnsProvider: 'cloudflare',
    dnsApiKey: '',
    dnsApiSecret: '',
    dnsApiToken: '',
    dnsZoneId: '',
    certPath: '',
    keyPath: ''
  });

  let domainsInput = $state('');

  onMount(async () => {
    try {
      const data = await client.get('/settings/certificates/status') as CertStatus;
      status = data;
      config.mode = data.mode;
      domainsInput = data.domains.join(', ');
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load certificate status';
    } finally {
      loading = false;
    }
  });

  $effect(() => {
    config.domains = domainsInput.split(',').map(d => d.trim()).filter(d => d.length > 0);
  });

  function getDaysRemainingColor(days: number): string {
    if (days <= 0) return 'var(--tcs-error)';
    if (days <= 7) return 'var(--tcs-error)';
    if (days <= 30) return 'var(--tcs-warning)';
    return 'var(--tcs-success)';
  }

  async function applyConfig() {
    saving = true;
    error = '';
    successMsg = '';
    try {
      // Backend expects nested letsencrypt / self_signed / provided objects
      const body: Record<string, unknown> = {
        mode: config.mode,
        domains: config.domains,
      };
      if (config.mode === 'letsencrypt') {
        body.letsencrypt = {
          email: config.adminEmail || '',
          challenge_type: config.challengeType || 'http-01',
          dns_provider:
            config.challengeType === 'dns-01'
              ? {
                  provider: config.dnsProvider || 'cloudflare',
                  api_key: config.dnsApiKey || '',
                  api_secret: config.dnsApiSecret || '',
                  api_token: config.dnsApiToken || '',
                  zone_id: config.dnsZoneId || '',
                }
              : null,
        };
      } else if (config.mode === 'self-signed') {
        body.self_signed = { domains: config.domains };
      } else if (config.mode === 'provided') {
        body.provided = {
          cert_path: config.certPath || '',
          key_path: config.keyPath || '',
        };
      }

      const res = (await client.put('/settings/certificates/config', body)) as {
        message?: string;
        restartRequired?: boolean;
        overlayPath?: string;
      };
      successMsg =
        res.message ||
        'TLS config saved. Restart TCS (systemctl restart tcs) for certificates to take effect.';
      success(successMsg);
      // Refresh status (still shows old mode until restart)
      try {
        status = (await client.get('/settings/certificates/status')) as CertStatus;
      } catch {
        /* ignore */
      }
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to apply configuration';
      notifyError(error);
    } finally {
      saving = false;
    }
  }

  async function renewNow() {
    renewing = true;
    error = '';
    successMsg = '';
    try {
      await client.post('/settings/certificates/renew');
      successMsg =
        'Renewal requested for the currently loaded TLS mode. If you just switched to Let\'s Encrypt, restart TCS first so the new mode is active.';
      success(successMsg);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to renew certificate';
      notifyError(error);
    } finally {
      renewing = false;
    }
  }
</script>

<div class="cert-page">
  <div class="page-header">
    <h1>SSL/TLS Certificates</h1>
    <p class="description">
      Manage TLS for the TCS HTTPS listener. Mode is loaded at process start — after saving, run
      <code>systemctl restart tcs</code>. Let's Encrypt HTTP-01 needs port 80 reachable from the
      public internet on the configured domain(s).
    </p>
  </div>

  {#if loading}
    <Spinner />
  {:else}
    <div class="cert-grid">
      {#if status}
        <div class="status-card">
          <h2>Certificate Status</h2>
          <div class="status-details">
            <div class="status-row">
              <span class="label">Issuer</span>
              <span class="value">{status.issuer || 'None'}</span>
            </div>
            <div class="status-row">
              <span class="label">Domains</span>
              <span class="value">{status.domains.length > 0 ? status.domains.join(', ') : 'None configured'}</span>
            </div>
            <div class="status-row">
              <span class="label">Expiry</span>
              <span class="value">{status.expiryDate ? new Date(status.expiryDate).toLocaleDateString() : 'N/A'}</span>
            </div>
            <div class="status-row">
              <span class="label">Days Remaining</span>
              <span class="value" style="color: {getDaysRemainingColor(status.daysRemaining)}">
                {status.daysRemaining >= 0 ? `${status.daysRemaining} days` : 'Expired'}
              </span>
            </div>
            <div class="status-row">
              <span class="label">Mode</span>
              <span class="value mode-badge">{status.mode}</span>
            </div>
          </div>
        </div>
      {/if}

      <div class="config-card">
        <h2>Certificate Configuration</h2>

        {#if error}
          <div class="error-banner">{error}</div>
        {/if}

        {#if successMsg}
          <div class="success-banner">{successMsg}</div>
        {/if}

        <div class="form-group">
          <label for="mode">Certificate Mode</label>
          <select id="mode" bind:value={config.mode}>
            <option value="letsencrypt">Let's Encrypt (Automated)</option>
            <option value="self-signed">Self-Signed</option>
            <option value="provided">Provided (Upload)</option>
            <option value="disabled">Disabled</option>
          </select>
        </div>

        {#if config.mode === 'letsencrypt'}
          <div class="section-divider">
            <h3>Let's Encrypt Configuration</h3>
          </div>

          <div class="form-group">
            <label for="domains">Domains</label>
            <input
              id="domains"
              type="text"
              bind:value={domainsInput}
              placeholder="example.com, api.example.com"
            />
            <span class="hint">Comma-separated list of domains to secure</span>
          </div>

          <div class="form-group">
            <label for="adminEmail">Admin Email</label>
            <input
              id="adminEmail"
              type="email"
              bind:value={config.adminEmail}
              placeholder="admin@example.com"
            />
            <span class="hint">Used for Let's Encrypt account and expiry notifications</span>
          </div>

          <div class="form-group">
            <label for="challengeType">Challenge Type</label>
            <select id="challengeType" bind:value={config.challengeType}>
              <option value="http-01">HTTP-01 (Port 80)</option>
              <option value="dns-01">DNS-01 (DNS TXT record)</option>
            </select>
          </div>

          {#if config.challengeType === 'dns-01'}
            <div class="dns-section">
              <div class="form-group">
                <label for="dnsProvider">DNS Provider</label>
                <select id="dnsProvider" bind:value={config.dnsProvider}>
                  <option value="cloudflare">Cloudflare</option>
                  <option value="godaddy">GoDaddy</option>
                  <option value="route53">AWS Route53</option>
                </select>
              </div>

              {#if config.dnsProvider === 'cloudflare'}
                <div class="form-group">
                  <label for="dnsApiToken">API Token</label>
                  <input
                    id="dnsApiToken"
                    type="password"
                    bind:value={config.dnsApiToken}
                    placeholder="Cloudflare API Token"
                  />
                </div>
                <div class="form-group">
                  <label for="dnsZoneId">Zone ID</label>
                  <input
                    id="dnsZoneId"
                    type="text"
                    bind:value={config.dnsZoneId}
                    placeholder="Cloudflare Zone ID"
                  />
                </div>
              {:else if config.dnsProvider === 'godaddy'}
                <div class="form-group">
                  <label for="dnsApiKey">API Key</label>
                  <input
                    id="dnsApiKey"
                    type="password"
                    bind:value={config.dnsApiKey}
                    placeholder="GoDaddy API Key"
                  />
                </div>
                <div class="form-group">
                  <label for="dnsApiSecret">API Secret</label>
                  <input
                    id="dnsApiSecret"
                    type="password"
                    bind:value={config.dnsApiSecret}
                    placeholder="GoDaddy API Secret"
                  />
                </div>
              {:else if config.dnsProvider === 'route53'}
                <div class="form-group">
                  <label for="dnsApiKey">AWS Access Key ID</label>
                  <input
                    id="dnsApiKey"
                    type="password"
                    bind:value={config.dnsApiKey}
                    placeholder="AWS Access Key ID"
                  />
                </div>
                <div class="form-group">
                  <label for="dnsApiSecret">AWS Secret Access Key</label>
                  <input
                    id="dnsApiSecret"
                    type="password"
                    bind:value={config.dnsApiSecret}
                    placeholder="AWS Secret Access Key"
                  />
                </div>
              {/if}
            </div>
          {/if}
        {:else if config.mode === 'self-signed'}
          <div class="section-divider">
            <h3>Self-Signed Configuration</h3>
          </div>

          <div class="form-group">
            <label for="domains-signed">Domains</label>
            <input
              id="domains-signed"
              type="text"
              bind:value={domainsInput}
              placeholder="localhost, tcs.internal"
            />
            <span class="hint">Comma-separated list of domains for the self-signed certificate</span>
          </div>
        {:else if config.mode === 'provided'}
          <div class="section-divider">
            <h3>Upload Certificates</h3>
          </div>

          <div class="form-group">
            <label for="certPath">Certificate File Path</label>
            <input
              id="certPath"
              type="text"
              bind:value={config.certPath}
              placeholder="/path/to/certificate.pem"
            />
            <span class="hint">Path to the certificate file on the server</span>
          </div>

          <div class="form-group">
            <label for="keyPath">Private Key File Path</label>
            <input
              id="keyPath"
              type="text"
              bind:value={config.keyPath}
              placeholder="/path/to/private-key.pem"
            />
            <span class="hint">Path to the private key file on the server</span>
          </div>
        {/if}

        <div class="actions">
          <Button variant="primary" onclick={applyConfig} disabled={saving}>
            {saving ? 'Applying...' : 'Apply'}
          </Button>
          {#if config.mode === 'letsencrypt'}
            <Button variant="secondary" onclick={renewNow} disabled={renewing}>
              {renewing ? 'Renewing...' : 'Renew Now'}
            </Button>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .cert-page h1 { margin: 0 0 0.5rem; }
  .description { color: var(--tcs-text-muted); margin: 0; }
  .page-header { margin-bottom: 1.5rem; }

  .cert-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
  }
  @media (max-width: 900px) {
    .cert-grid { grid-template-columns: 1fr; }
  }

  .status-card, .config-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
  }

  .status-card h2, .config-card h2 {
    font-size: 1rem;
    margin: 0 0 1rem;
  }

  .status-details {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .status-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .status-row .label {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }

  .status-row .value {
    font-size: 0.875rem;
    font-weight: 500;
  }

  .mode-badge {
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    background: rgba(79, 139, 255, 0.15);
    color: var(--tcs-secondary);
    text-transform: capitalize;
  }

  .form-group {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }

  .form-group label {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }

  .form-group input, .form-group select {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    outline: none;
  }

  .form-group input:focus, .form-group select:focus {
    border-color: var(--tcs-primary);
  }

  .form-group select {
    cursor: pointer;
  }

  .hint {
    font-size: 0.75rem;
    color: var(--tcs-text-muted);
  }

  .section-divider {
    margin: 1.5rem 0 1rem;
  }

  .section-divider h3 {
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
    margin: 0;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--tcs-border);
  }

  .dns-section {
    padding: 1rem;
    background: var(--tcs-background);
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
  }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-error);
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .success-banner {
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-success);
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .actions {
    display: flex;
    gap: 0.75rem;
    margin-top: 1.5rem;
  }
</style>
