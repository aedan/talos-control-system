<script lang="ts">
  import Logo from '$lib/branding/components/Logo.svelte';
  import Button from '$lib/components/Button.svelte';
  
  let email = '';
  let password = '';
  let error = '';
  let loading = false;
  
  async function handleLogin(e: Event) {
    e.preventDefault();
    error = '';
    loading = true;
    try {
      const res = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, password })
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.message || 'Authentication failed');
      }
      const data = await res.json();
      localStorage.setItem('tcs_token', data.token);
      window.location.href = '/';
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Authentication failed';
    } finally {
      loading = false;
    }
  }
</script>

<div class="login-page">
  <div class="login-card">
    <div class="login-header">
      <Logo size="md" />
      <p class="tagline">Kubernetes Management Simplified</p>
    </div>
    
    <form class="login-form" onsubmit={handleLogin}>
      {#if error}
        <div class="error-banner">{error}</div>
      {/if}
      
      <div class="form-group">
        <label for="email">Email</label>
        <input
          id="email"
          type="email"
          bind:value={email}
          placeholder="admin@example.com"
          autocomplete="email"
          required
        />
      </div>
      
      <div class="form-group">
        <label for="password">Password</label>
        <input
          id="password"
          type="password"
          bind:value={password}
          placeholder="Enter your password"
          autocomplete="current-password"
          required
        />
      </div>
      
      <Button
        variant="primary"
        size="lg"
        type="submit"
        disabled={loading}
        class="submit-btn"
      >
        {loading ? 'Signing in...' : 'Sign In'}
      </Button>
    </form>
    
    <div class="divider">
      <span>or continue with</span>
    </div>
    
    <div class="alt-auth">
      <a href="/api/auth/oidc" class="alt-btn oidc">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
          <path d="M12 6v6l4 2"/>
        </svg>
        Single Sign-On
      </a>
      <a href="/api/auth/saml" class="alt-btn saml">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="3" y="11" width="18" height="11" rx="2" ry="2"/>
          <path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        </svg>
        SAML
      </a>
    </div>
  </div>
</div>

<style>
  .login-page {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--tcs-background);
  }
  
  .login-card {
    width: 100%;
    max-width: 400px;
    padding: 2.5rem;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 12px;
  }
  
  .login-header {
    text-align: center;
    margin-bottom: 2rem;
  }
  
  .login-header .tagline {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
    margin-top: 0.75rem;
  }
  
  .login-form {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  
  .form-group label {
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }
  
  .form-group input {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.6rem 0.8rem;
    color: var(--tcs-text);
    outline: none;
    transition: border-color 0.15s;
  }
  
  .form-group input:focus {
    border-color: var(--tcs-primary);
  }
  
  .error-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 0.75rem;
    color: var(--tcs-error);
    font-size: 0.875rem;
  }
  
  .submit-btn {
    width: 100%;
    margin-top: 0.5rem;
  }
  
  .divider {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin: 1.5rem 0;
    color: var(--tcs-text-muted);
    font-size: 0.8rem;
  }
  
  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--tcs-border);
  }
  
  .alt-auth {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  
  .alt-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.6rem;
    border-radius: 6px;
    border: 1px solid var(--tcs-border);
    color: var(--tcs-text);
    font-size: 0.875rem;
    transition: all 0.15s;
    text-decoration: none;
  }
  
  .alt-btn:hover {
    background: var(--tcs-surface-hover);
    border-color: var(--tcs-secondary);
    text-decoration: none;
  }
  
  .alt-btn.oidc {
    border-color: rgba(79, 139, 255, 0.3);
  }
  
  .alt-btn.saml {
    border-color: rgba(160, 160, 160, 0.3);
  }
</style>
