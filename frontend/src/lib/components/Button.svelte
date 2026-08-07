<script lang="ts">
  export let variant: 'primary' | 'secondary' | 'ghost' | 'danger' = 'primary';
  export let size: 'sm' | 'md' | 'lg' = 'md';
  export let disabled = false;
  
  const variantClasses = {
    primary: 'bg-primary text-white hover:brightness-110 border border-transparent',
    secondary: 'bg-surface text-text hover:bg-surface-hover border border-border',
    ghost: 'text-text-muted hover:text-text hover:bg-white/5 border border-transparent',
    danger: 'bg-error text-white hover:brightness-110 border border-transparent',
  };
  
  const sizeClasses = { sm: 'px-3 py-1.5 text-sm', md: 'px-4 py-2', lg: 'px-6 py-3 text-lg' };
</script>

<button class="btn {variantClasses[variant]} {sizeClasses[size]}"
        {disabled}
        class:opacity-50={disabled}
        class:pointer-events-none={disabled}>
  <slot />
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    border-radius: 6px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }
</style>
