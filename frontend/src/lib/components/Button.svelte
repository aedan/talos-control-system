<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    variant = 'primary',
    size = 'md',
    disabled = false,
    type = 'button',
    onclick,
    children,
    class: extraClass = '',
    title
  }: {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md' | 'lg';
    disabled?: boolean;
    type?: 'button' | 'submit' | 'reset';
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
    class?: string;
    title?: string;
  } = $props();

  const variantClasses = {
    primary: 'bg-primary text-white hover:brightness-110 border border-transparent',
    secondary: 'bg-surface text-text hover:bg-surface-hover border border-border',
    ghost: 'text-text-muted hover:text-text hover:bg-white/5 border border-transparent',
    danger: 'bg-error text-white hover:brightness-110 border border-transparent',
  };

  const sizeClasses = { sm: 'px-3 py-1.5 text-sm', md: 'px-4 py-2', lg: 'px-6 py-3 text-lg' };
</script>

<button class="btn {variantClasses[variant]} {sizeClasses[size]} {extraClass}"
        {type}
        {disabled}
        {onclick}
        {title}
        class:opacity-50={disabled}
        class:pointer-events-none={disabled}>
  {#if children}
    {@render children()}
  {/if}
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
