<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface GroupRoleMapping {
    groupDnPattern: string;
    role: 'reader' | 'operator' | 'admin';
  }

  interface AuthConfig {
    ldap: {
      enabled: boolean;
      serverUrl: string;
      bindDn: string;
      bindPassword: string;
      userSearchBase: string;
      userSearchFilter: string;
      defaultRole: 'reader' | 'operator' | 'admin';
      groupRoleMappings: GroupRoleMapping[];
    };
    oidc: {
      enabled: boolean;
      issuerUrl: string;
      clientId: string;
      clientSecret: string;
      redirectUrl: string;
      scopes: string;
    };
  }

  interface PasswordChange {
    currentPassword: string;
    newPassword: string;
    confirmPassword: string;
  }

  let loading = $state(true);
  let saving = $state(false);
  let changingPassword = $state(false);
  let error = $state('');
  let successMsg = $state('');
  let passwordError = $state('');
  let passwordSuccess = $state('');

  let config = $state<AuthConfig>({
    ldap: {
      enabled: false,
      serverUrl: '',
      bindDn: '',
      bindPassword: '',
      userSearchBase: '',
      userSearchFilter: '(mail={})',
      defaultRole: 'reader',
      groupRoleMappings: []
    },
    oidc: {
      enabled: false,
      issuerUrl: '',
      clientId: '',
      clientSecret: '',
      redirectUrl: '',
      scopes: 'openid,profile,email'
    }
  });

  let passwordForm = $state<PasswordChange>({
    currentPassword: '',
    newPassword: '',
    confirmPassword: ''
  });

  onMount(async () => {
    try {
      const data = await client.get('/settings/auth/config') as AuthConfig;
      if (data) {
        config = data;
      }
    } catch {
      // Use defaults if endpoint doesn't exist yet
    } finally {
      loading = false;
    }
  });

  function addGroupMapping() {
    config.ldap.groupRoleMappings.push({
      groupDnPattern: '',
      role: 'reader'
    });
  }

  function removeGroupMapping(index: number) {
    config.ldap.groupRoleMappings.splice(index, 1);
  }

  async function changePassword() {
    passwordError = '';
    passwordSuccess = '';
    changingPassword = true;

    if (passwordForm.newPassword !== passwordForm.confirmPassword) {
      passwordError = 'New passwords do not match';
      changingPassword = false;
      return;
    }

    if (passwordForm.newPassword.length < 8) {
      passwordError = 'Password must be at least 8 characters';
      changingPassword = false;
      return;
    }

    try {
      await client.post('/api/auth/password', {
        current_password: passwordForm.currentPassword,
        new_password: passwordForm.newPassword
      });
      passwordSuccess = 'Password changed successfully';
      passwordForm.currentPassword = '';
      passwordForm.newPassword = '';
      passwordForm.confirmPassword = '';
      success('Password changed successfully');
    } catch (e: unknown) {
      passwordError = e instanceof Error ? e.message : 'Failed to change password';
      notifyError('Failed to change password');
    } finally {
      changingPassword = false;
    }
  }

  async function saveConfig() {
    saving = true;
    error = '';
    successMsg = '';
    try {
      await client.put('/settings/auth/config', config);
      successMsg = 'Authentication configuration saved successfully';
      success('Authentication configuration saved');
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to save configuration';
      notifyError('Failed to save authentication configuration');
    } finally {
      saving = false;
    }
  }
</script>

<div class="auth-page">
  <div class="page-header">
    <h1>Authentication</h1>
    <p class="description">Configure password policies, LDAP, and OIDC authentication.</p>
  </div>

  {#if loading}
    <Spinner />
  {:else}
    <div class="auth-grid">
      <div class="password-card">
        <h2>Change Password</h2>
        <p class="section-desc">Update your current account password.</p>

        {#if passwordError}
          <div class="error-banner">{passwordError}</div>
        {/if}

        {#if passwordSuccess}
          <div class="success-banner">{passwordSuccess}</div>
        {/if}

        <div class="form-group">
          <label for="currentPassword">Current Password</label>
          <input
            id="currentPassword"
            type="password"
            bind:value={passwordForm.currentPassword}
            autocomplete="current-password"
          />
        </div>

        <div class="form-group">
          <label for="newPassword">New Password</label>
          <input
            id="newPassword"
            type="password"
            bind:value={passwordForm.newPassword}
            autocomplete="new-password"
          />
        </div>

        <div class="form-group">
          <label for="confirmPassword">Confirm New Password</label>
          <input
            id="confirmPassword"
            type="password"
            bind:value={passwordForm.confirmPassword}
            autocomplete="new-password"
          />
        </div>

        <Button variant="primary" onclick={changePassword} disabled={changingPassword}>
          {changingPassword ? 'Changing...' : 'Change Password'}
        </Button>
      </div>

      <div class="auth-config">
        {#if error}
          <div class="error-banner">{error}</div>
        {/if}

        {#if successMsg}
          <div class="success-banner">{successMsg}</div>
        {/if}

        <div class="ldap-card">
          <div class="card-header">
            <h2>LDAP / Active Directory</h2>
            <label class="toggle">
              <input type="checkbox" checked={config.ldap.enabled} onchange={() => config.ldap.enabled = !config.ldap.enabled} />
              <span class="toggle-track">
                <span class="toggle-thumb"></span>
              </span>
            </label>
          </div>

          {#if config.ldap.enabled}
            <div class="form-group">
              <label for="ldapServer">Server URL</label>
              <input
                id="ldapServer"
                type="text"
                bind:value={config.ldap.serverUrl}
                placeholder="ldap://dc.example.com:389"
              />
            </div>

            <div class="form-group">
              <label for="bindDn">Bind DN</label>
              <input
                id="bindDn"
                type="text"
                bind:value={config.ldap.bindDn}
                placeholder="cn=binduser,ou=users,dc=example,dc=com"
              />
            </div>

            <div class="form-group">
              <label for="bindPassword">Bind Password</label>
              <input
                id="bindPassword"
                type="password"
                bind:value={config.ldap.bindPassword}
                placeholder="Bind account password"
              />
            </div>

            <div class="form-group">
              <label for="userSearchBase">User Search Base</label>
              <input
                id="userSearchBase"
                type="text"
                bind:value={config.ldap.userSearchBase}
                placeholder="ou=users,dc=example,dc=com"
              />
            </div>

            <div class="form-group">
              <label for="userSearchFilter">User Search Filter</label>
              <input
                id="userSearchFilter"
                type="text"
                bind:value={config.ldap.userSearchFilter}
                placeholder="(mail=USERNAME)"
              />
              <span class="hint">Use {user} or USERNAME in the filter for the username placeholder</span>
            </div>

            <div class="form-group">
              <label for="defaultRole">Default Role</label>
              <select id="defaultRole" bind:value={config.ldap.defaultRole}>
                <option value="reader">Reader</option>
                <option value="operator">Operator</option>
                <option value="admin">Admin</option>
              </select>
            </div>

            <div class="group-mappings">
              <div class="mapping-header">
                <h3>Group-to-Role Mappings</h3>
                <Button variant="ghost" size="sm" onclick={addGroupMapping}>+ Add</Button>
              </div>

              {#each config.ldap.groupRoleMappings as mapping, index}
                <div class="mapping-row">
                  <div class="form-group mapping-input">
                    <input
                      type="text"
                      bind:value={mapping.groupDnPattern}
                      placeholder="Group DN pattern (e.g., cn=admins,ou=groups,dc=example,dc=com)"
                    />
                  </div>
                  <select bind:value={mapping.role}>
                    <option value="reader">Reader</option>
                    <option value="operator">Operator</option>
                    <option value="admin">Admin</option>
                  </select>
                  <button class="remove-btn" onclick={() => removeGroupMapping(index)} title="Remove mapping">
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
                      <line x1="18" y1="6" x2="6" y2="18"/>
                      <line x1="6" y1="6" x2="18" y2="18"/>
                    </svg>
                  </button>
                </div>
              {/each}

              {#if config.ldap.groupRoleMappings.length === 0}
                <p class="empty-mappings">No group mappings configured. Add mappings to assign roles based on group membership.</p>
              {/if}
            </div>
          {/if}
        </div>

        <div class="oidc-card">
          <div class="card-header">
            <h2>OpenID Connect (OIDC)</h2>
            <label class="toggle">
              <input type="checkbox" checked={config.oidc.enabled} onchange={() => config.oidc.enabled = !config.oidc.enabled} />
              <span class="toggle-track">
                <span class="toggle-thumb"></span>
              </span>
            </label>
          </div>

          {#if config.oidc.enabled}
            <div class="form-group">
              <label for="issuerUrl">Issuer URL</label>
              <input
                id="issuerUrl"
                type="url"
                bind:value={config.oidc.issuerUrl}
                placeholder="https://auth.example.com/realms/tcs"
              />
            </div>

            <div class="form-group">
              <label for="clientId">Client ID</label>
              <input
                id="clientId"
                type="text"
                bind:value={config.oidc.clientId}
                placeholder="tcs-client"
              />
            </div>

            <div class="form-group">
              <label for="clientSecret">Client Secret</label>
              <input
                id="clientSecret"
                type="password"
                bind:value={config.oidc.clientSecret}
                placeholder="OIDC client secret"
              />
            </div>

            <div class="form-group">
              <label for="redirectUrl">Redirect URL</label>
              <input
                id="redirectUrl"
                type="url"
                bind:value={config.oidc.redirectUrl}
                placeholder="https://tcs.example.com/api/auth/oidc/callback"
              />
            </div>

            <div class="form-group">
              <label for="scopes">Scopes</label>
              <input
                id="scopes"
                type="text"
                bind:value={config.oidc.scopes}
                placeholder="openid,profile,email"
              />
              <span class="hint">Comma-separated OAuth scopes</span>
            </div>
          {/if}
        </div>

        <Button variant="primary" onclick={saveConfig} disabled={saving}>
          {saving ? 'Saving...' : 'Save Configuration'}
        </Button>
      </div>
    </div>
  {/if}
</div>

<style>
  .auth-page h1 { margin: 0 0 0.5rem; }
  .description { color: var(--tcs-text-muted); margin: 0; }
  .page-header { margin-bottom: 1.5rem; }

  .auth-grid {
    display: grid;
    grid-template-columns: 380px 1fr;
    gap: 1.5rem;
  }
  @media (max-width: 1000px) {
    .auth-grid { grid-template-columns: 1fr; }
  }

  .password-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .password-card h2 {
    font-size: 1rem;
    margin: 0;
  }

  .section-desc {
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
    margin: 0;
  }

  .auth-config {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }

  .ldap-card, .oidc-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }

  .ldap-card h2, .oidc-card h2 {
    font-size: 1rem;
    margin: 0;
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

  .toggle {
    cursor: pointer;
    display: flex;
    align-items: center;
  }
  .toggle input { display: none; }
  .toggle-track {
    width: 36px;
    height: 20px;
    background: var(--tcs-border);
    border-radius: 10px;
    position: relative;
    transition: background 0.15s;
  }
  .toggle input:checked + .toggle-track {
    background: var(--tcs-success);
  }
  .toggle-thumb {
    width: 16px;
    height: 16px;
    background: white;
    border-radius: 50%;
    position: absolute;
    top: 2px;
    left: 2px;
    transition: left 0.15s;
  }
  .toggle input:checked + .toggle-track .toggle-thumb {
    left: 18px;
  }

  .group-mappings {
    margin-top: 0.5rem;
    padding: 1rem;
    background: var(--tcs-background);
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
  }

  .mapping-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  .mapping-header h3 {
    font-size: 0.875rem;
    color: var(--tcs-text-muted);
    margin: 0;
  }

  .mapping-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }

  .mapping-input {
    flex: 1;
    margin-bottom: 0;
  }

  .mapping-input input {
    width: 100%;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    color: var(--tcs-text);
    outline: none;
    font-size: 0.875rem;
  }

  .mapping-input input:focus {
    border-color: var(--tcs-primary);
  }

  .mapping-row select {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    color: var(--tcs-text);
    outline: none;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .remove-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 4px;
    border: 1px solid var(--tcs-border);
    background: transparent;
    color: var(--tcs-text-muted);
    cursor: pointer;
    transition: all 0.15s;
  }

  .remove-btn:hover {
    background: rgba(239, 68, 68, 0.1);
    color: var(--tcs-error);
    border-color: rgba(239, 68, 68, 0.3);
  }

  .empty-mappings {
    font-size: 0.8rem;
    color: var(--tcs-text-muted);
    text-align: center;
    padding: 1rem 0;
    margin: 0;
  }

  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-error);
    font-size: 0.875rem;
  }

  .success-banner {
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-success);
    font-size: 0.875rem;
  }
</style>
