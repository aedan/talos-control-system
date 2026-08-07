<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  
  interface User {
    id: string;
    email: string;
    displayName: string;
    role: 'admin' | 'editor' | 'reader';
    isActive: boolean;
    lastLogin: string | null;
    createdAt: string;
  }
  
  let users = $state<User[]>([]);
  let loading = $state(true);
  let error = $state('');
  
  onMount(async () => {
    try {
      users = await client.get('/users') as User[];
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load users';
    } finally {
      loading = false;
    }
  });
  
  async function toggleActive(user: User) {
    try {
      await client.put(`/users/${user.id}`, { is_active: !user.isActive });
      user.isActive = !user.isActive;
      success(`User ${user.isActive ? 'activated' : 'deactivated'}`);
    } catch {
      notifyError('Failed to update user');
    }
  }
  
  async function deleteUser(user: User) {
    if (!confirm(`Delete user "${user.displayName}"?`)) return;
    try {
      await client.delete(`/users/${user.id}`);
      users = users.filter(u => u.id !== user.id);
      success('User deleted');
    } catch {
      notifyError('Failed to delete user');
    }
  }
</script>

<div class="users-page">
  <div class="page-header">
    <h1>Users</h1>
    <Button variant="primary">Add User</Button>
  </div>
  
  {#if loading}
    <Spinner />
  {:else if error}
    <div class="error">{error}</div>
  {:else if users.length === 0}
    <div class="empty-state">
      <p>No users configured yet</p>
      <p class="hint">Add users or configure an identity provider for authentication.</p>
    </div>
  {:else}
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Email</th>
          <th>Role</th>
          <th>Status</th>
          <th>Last Login</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each users as user (user.id)}
          <tr>
            <td>{user.displayName}</td>
            <td>{user.email}</td>
            <td><span class="role-badge">{user.role}</span></td>
            <td>
              <label class="toggle">
                <input type="checkbox" checked={user.isActive} on:change={() => toggleActive(user)} />
                <span class="toggle-track">
                  <span class="toggle-thumb"></span>
                </span>
              </label>
            </td>
            <td>{user.lastLogin ? new Date(user.lastLogin).toLocaleDateString() : 'Never'}</td>
            <td>
              <div class="actions">
                <Button variant="ghost" size="sm">Edit</Button>
                <Button variant="danger" size="sm" onclick={() => deleteUser(user)}>Delete</Button>
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .users-page h1 { margin: 0; }
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.5rem;
  }
  .error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    color: var(--tcs-error);
  }
  .empty-state {
    text-align: center;
    padding: 3rem;
    color: var(--tcs-text-muted);
  }
  .empty-state .hint {
    font-size: 0.875rem;
    margin-top: 0.5rem;
  }
  
  .data-table {
    width: 100%;
    border-collapse: collapse;
  }
  .data-table th, .data-table td {
    text-align: left;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--tcs-border);
  }
  .data-table th {
    color: var(--tcs-text-muted);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .data-table tr:hover {
    background: var(--tcs-surface-hover);
  }
  
  .role-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    display: inline-block;
    text-transform: capitalize;
  }
  .role-badge.admin { background: rgba(239, 68, 68, 0.15); color: var(--tcs-error); }
  .role-badge.editor { background: rgba(79, 139, 255, 0.15); color: var(--tcs-secondary); }
  .role-badge.reader { background: rgba(160, 160, 160, 0.15); color: var(--tcs-text-muted); }
  
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
  
  .actions {
    display: flex;
    gap: 0.5rem;
  }
</style>
