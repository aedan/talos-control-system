<script lang="ts">
</script>

<div class="settings-page">
  <h1>Settings</h1>
  <p class="description">Manage your TCS deployment configuration.</p>
  
  <div class="settings-grid">
    <a href="/settings/branding" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 2.69l5.66 5.66a8 8 0 1 1-11.31 0z"/>
        </svg>
      </div>
      <h3>White-Label Branding</h3>
      <p>Customize colors, logo, and identity of your deployment.</p>
      <span class="card-arrow">&rarr;</span>
    </a>
    
    <a href="/settings/users" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/>
          <circle cx="9" cy="7" r="4"/>
          <path d="M23 21v-2a4 4 0 0 0-3-3.87"/>
          <path d="M16 3.13a4 4 0 0 1 0 7.75"/>
        </svg>
      </div>
      <h3>Users &amp; Roles</h3>
      <p>Manage user accounts and access control.</p>
      <span class="card-arrow">&rarr;</span>
    </a>
    
    <a href="/settings/certificates" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
        </svg>
      </div>
      <h3>SSL/TLS Certificates</h3>
      <p>Manage TLS certificates and Let's Encrypt integration.</p>
      <span class="card-arrow">&rarr;</span>
    </a>
    
    <a href="/settings/auth" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/>
          <circle cx="9" cy="7" r="4"/>
          <path d="M23 21v-2a4 4 0 0 0-3-3.87"/>
          <path d="M16 3.13a4 4 0 0 1 0 7.75"/>
        </svg>
      </div>
      <h3>Authentication</h3>
      <p>Configure OIDC, LDAP, SAML status, and local auth.</p>
      <span class="card-arrow">&rarr;</span>
    </a>

    <a href="/settings/siderolink" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/>
          <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>
        </svg>
      </div>
      <h3>Siderolink</h3>
      <p>Join tokens and registered peer inventory (WG later).</p>
      <span class="card-arrow">&rarr;</span>
    </a>

    <a href="/settings/audit-logs" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
        </svg>
      </div>
      <h3>Audit Logs</h3>
      <p>View and export system audit trail.</p>
      <span class="card-arrow">&rarr;</span>
    </a>

    <a href="/settings/system" class="settings-card">
      <div class="card-icon">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
      </div>
      <h3>System</h3>
      <p>Database, networking, and runtime configuration.</p>
      <span class="card-arrow">&rarr;</span>
    </a>
  </div>
</div>

<style>
  .settings-page h1 { margin: 0 0 0.25rem; }
  .description { color: var(--tcs-text-muted); margin: 0 0 2rem; }
  
  .settings-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 1rem;
  }
  
  .settings-card {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    transition: all 0.15s;
    text-decoration: none;
    color: inherit;
    position: relative;
  }
  
  .settings-card:hover {
    background: var(--tcs-surface-hover);
    border-color: var(--tcs-primary);
    text-decoration: none;
  }
  
  .settings-card.disabled {
    opacity: 0.5;
    cursor: default;
  }
  
  .settings-card.disabled:hover {
    background: var(--tcs-surface);
    border-color: var(--tcs-border);
  }
  
  .card-icon {
    color: var(--tcs-secondary);
    margin-bottom: 0.5rem;
  }
  
  .settings-card h3 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }
  
  .settings-card p {
    margin: 0;
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
    line-height: 1.5;
  }
  
  .card-arrow {
    position: absolute;
    top: 1.5rem;
    right: 1.5rem;
    color: var(--tcs-text-muted);
    font-size: 1.2rem;
    transition: color 0.15s;
  }
  
  .settings-card:hover .card-arrow {
    color: var(--tcs-secondary);
  }
  
  .card-badge {
    position: absolute;
    top: 1rem;
    right: 1rem;
    font-size: 0.65rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: rgba(245, 158, 11, 0.15);
    color: var(--tcs-warning);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
</style>
