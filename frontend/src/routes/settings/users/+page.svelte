<script lang="ts">
  import { onMount } from 'svelte';
  import { client } from '$lib/api/client';
  import { success, error as notifyError } from '$lib/stores/notifications';
  import Button from '$lib/components/Button.svelte';
  import Spinner from '$lib/components/Spinner.svelte';

  interface User {
    id: string;
    email: string;
    display_name: string;
    role: 'admin' | 'editor' | 'reader';
    is_active: boolean;
    last_login: string | null;
    created_at: string;
  }

  let users = $state<User[]>([]);
  let loading = $state(true);
  let error = $state('');
  let showAddForm = $state(false);
  let editingUser = $state<User | null>(null);

  let addForm = $state({
    email: '',
    display_name: '',
    role: 'reader' as 'admin' | 'editor' | 'reader',
    password: '',
  });

  let editForm = $state({
    display_name: '',
    role: 'reader' as 'admin' | 'editor' | 'reader',
    password: '',
  });

  async function loadUsers() {
    loading = true;
    error = '';
    try {
      const data = await client.get('/users') as { users: User[] };
      users = data.users;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Failed to load users';
    } finally {
      loading = false;
    }
  }

  async function handleAddUser() {
    if (!addForm.email || !addForm.display_name) {
      notifyError('Email and display name are required');
      return;
    }
    try {
      await client.post('/users', {
        email: addForm.email,
        display_name: addForm.display_name,
        role: addForm.role,
        password: addForm.password || undefined,
      });
      addForm = { email: '', display_name: '', role: 'reader', password: '' };
      showAddForm = false;
      success('User created');
      await loadUsers();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to create user');
    }
  }

  function startEdit(user: User) {
    editingUser = user;
    editForm = {
      display_name: user.display_name,
      role: user.role,
      password: '',
    };
  }

  async function handleEditUser() {
    if (!editingUser) return;
    try {
      await client.put(`/users/${editingUser.id}`, {
        display_name: editForm.display_name,
        role: editForm.role,
        password: editForm.password || undefined,
      });
      editingUser = null;
      success('User updated');
      await loadUsers();
    } catch (e: unknown) {
      notifyError(e instanceof Error ? e.message : 'Failed to update user');
    }
  }

  function cancelEdit() {
    editingUser = null;
  }

  async function toggleActive(user: User) {
    try {
      await client.put(`/users/${user.id}`, { is_active: !user.is_active });
      user.is_active = !user.is_active;
      success(`User ${user.is_active ? 'activated' : 'deactivated'}`);
    } catch {
      notifyError('Failed to update user');
    }
  }

  async function deleteUser(user: User) {
    if (!confirm(`Delete user "${user.display_name}"?`)) return;
    try {
      await client.delete(`/users/${user.id}`);
      users = users.filter(u => u.id !== user.id);
      success('User deleted');
    } catch {
      notifyError('Failed to delete user');
    }
  }

  function formatDate(ts: string | null): string {
    if (!ts) return 'Never';
    return new Date(ts).toLocaleDateString();
  }

  onMount(loadUsers);
</script>

<div class="users-page">
  <div class="page-header">
    <h1>Users</h1>
    <Button variant="primary" onclick={() => { showAddForm = !showAddForm; }}>
      {showAddForm ? 'Cancel' : 'Add User'}
    </Button>
  </div>

  {#if showAddForm}
    <div class="add-form">
      <h3>Create New User</h3>
      <div class="form-row">
        <div class="form-group">
          <label for="add-email">Email</label>
          <input id="add-email" type="email" bind:value={addForm.email} placeholder="user@example.com" required />
        </div>
        <div class="form-group">
          <label for="add-name">Display Name</label>
          <input id="add-name" type="text" bind:value={addForm.display_name} placeholder="John Doe" required />
        </div>
        <div class="form-group">
          <label for="add-role">Role</label>
          <select id="add-role" bind:value={addForm.role}>
            <option value="reader">Reader</option>
            <option value="editor">Editor</option>
            <option value="admin">Admin</option>
          </select>
        </div>
        <div class="form-group">
          <label for="add-password">Password</label>
          <input id="add-password" type="password" bind:value={addForm.password} placeholder="Leave empty for no password" />
        </div>
      </div>
      <div class="form-actions">
        <Button variant="primary" onclick={handleAddUser}>Create User</Button>
        <Button variant="ghost" onclick={() => showAddForm = false}>Cancel</Button>
      </div>
    </div>
  {/if}

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
          {#if editingUser && editingUser.id === user.id}
            <tr class="editing-row">
              <td colspan="6">
                <div class="edit-form-inline">
                  <div class="form-row">
                    <div class="form-group">
                      <label>Display Name</label>
                      <input type="text" bind:value={editForm.display_name} />
                    </div>
                    <div class="form-group">
                      <label>Role</label>
                      <select bind:value={editForm.role}>
                        <option value="reader">Reader</option>
                        <option value="editor">Editor</option>
                        <option value="admin">Admin</option>
                      </select>
                    </div>
                    <div class="form-group">
                      <label>New Password</label>
                      <input type="password" bind:value={editForm.password} placeholder="Leave empty to keep current" />
                    </div>
                  </div>
                  <div class="form-actions">
                    <Button variant="primary" size="sm" onclick={handleEditUser}>Save</Button>
                    <Button variant="ghost" size="sm" onclick={cancelEdit}>Cancel</Button>
                  </div>
                </div>
              </td>
            </tr>
          {:else}
            <tr>
              <td>{user.display_name}</td>
              <td>{user.email}</td>
              <td><span class="role-badge {user.role}">{user.role}</span></td>
              <td>
                <label class="toggle">
                  <input type="checkbox" checked={user.is_active} onchange={() => toggleActive(user)} />
                  <span class="toggle-track">
                    <span class="toggle-thumb"></span>
                  </span>
                </label>
              </td>
              <td>{formatDate(user.last_login)}</td>
              <td>
                <div class="actions">
                  <Button variant="ghost" size="sm" onclick={() => startEdit(user)}>Edit</Button>
                  <Button variant="danger" size="sm" onclick={() => deleteUser(user)}>Delete</Button>
                </div>
              </td>
            </tr>
          {/if}
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

  .add-form {
    background: var(--tcs-surface);
    border: 1px solid var(--tcs-border);
    border-radius: 8px;
    padding: 1.25rem;
    margin-bottom: 1.5rem;
  }
  .add-form h3 {
    margin: 0 0 1rem;
    font-size: 1rem;
  }

  .form-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 1rem;
  }
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .form-group label {
    color: var(--tcs-text-muted);
    font-size: 0.8rem;
  }
  .form-group input,
  .form-group select {
    background: var(--tcs-background);
    border: 1px solid var(--tcs-border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    color: var(--tcs-text);
    outline: none;
    font-size: 0.875rem;
  }
  .form-group input:focus,
  .form-group select:focus {
    border-color: var(--tcs-primary);
  }
  .form-actions {
    display: flex;
    gap: 0.5rem;
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
  .editing-row {
    background: var(--tcs-surface) !important;
  }

  .edit-form-inline {
    padding: 0.5rem;
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
