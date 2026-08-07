import { writable } from 'svelte/store';

export interface Notification {
  id: string;
  type: 'success' | 'error' | 'warning' | 'info';
  message: string;
  duration?: number;
}

export const notifications = writable<Notification[]>([]);

let idCounter = 0;

export function notify(
  message: string,
  type: Notification['type'] = 'info',
  duration = 5000
): void {
  const id = `n-${++idCounter}`;
  notifications.update((items) => [...items, { id, type, message, duration }]);
  
  if (duration > 0) {
    setTimeout(() => dismiss(id), duration);
  }
}

export function dismiss(id: string): void {
  notifications.update((items) => items.filter((n) => n.id !== id));
}

export function success(message: string, duration?: number): void {
  notify(message, 'success', duration);
}

export function error(message: string, duration?: number): void {
  notify(message, 'error', duration);
}

export function warning(message: string, duration?: number): void {
  notify(message, 'warning', duration);
}
