<script lang="ts">
</script>

<div class="machine-classes-page">
  <div class="page-header">
    <h1>Machine Classes</h1>
  </div>

  <div class="empty-state">
    <div class="icon">
      <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="var(--tcs-text-muted)" stroke-width="1.5">
        <rect x="2" y="3" width="20" height="14" rx="2"/>
        <path d="M8 21h8M12 17v4"/>
      </svg>
    </div>
    <h2>Machine classes coming soon</h2>
    <p>
      Machine classes allow you to define reusable machine configurations
      including resource allocations, security policies, and network settings.
      This feature will be available in the next release.
    </p>
    <div class="capabilities">
      <div class="capability">
        <span class="check">&#10005;</span>
        <span>Define reusable hardware profiles</span>
      </div>
      <div class="capability">
        <span class="check">&#10005;</span>
        <span>Set security policies per class</span>
      </div>
      <div class="capability">
        <span class="check">&#10005;</span>
        <span>Auto-assign machines to classes</span>
      </div>
    </div>
  </div>
</div>

<style>
  .machine-classes-page h1 { margin: 0; }
  .page-header { margin-bottom: 2rem; }
  
  .empty-state {
    text-align: center;
    padding: 4rem 2rem;
    max-width: 560px;
    margin: 0 auto;
  }
  
  .icon { margin-bottom: 1.5rem; }
  
  .empty-state h2 {
    margin: 0 0 0.75rem;
    font-size: 1.5rem;
    font-weight: 600;
  }
  
  .empty-state p {
    color: var(--tcs-text-muted);
    font-size: 0.95rem;
    line-height: 1.6;
    margin-bottom: 2rem;
  }
  
  .capabilities {
    text-align: left;
    display: inline-block;
  }
  
  .capability {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0;
    color: var(--tcs-text-muted);
    font-size: 0.875rem;
  }
  
  .check {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.6rem;
    flex-shrink: 0;
  }
</style>
