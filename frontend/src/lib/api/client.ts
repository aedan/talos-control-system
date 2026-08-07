const API_BASE = '/api';

export interface ApiError extends Error {
  status?: number;
}

export class TcsClient {
  constructor(private headers: Record<string, string> = {}) {}

  async get(path: string): Promise<unknown> {
    return this.request('GET', path);
  }

  async post(path: string, body?: unknown): Promise<unknown> {
    return this.request('POST', path, body);
  }

  async put(path: string, body?: unknown): Promise<unknown> {
    return this.request('PUT', path, body);
  }

  async delete(path: string): Promise<unknown> {
    return this.request('DELETE', path);
  }

  private async request(method: string, path: string, body?: unknown): Promise<unknown> {
    const opts: RequestInit = {
      method,
      headers: {
        'Content-Type': 'application/json',
        ...this.headers
      }
    };

    if (body && (method === 'POST' || method === 'PUT' || method === 'PATCH')) {
      opts.body = JSON.stringify(body);
    }

    const res = await fetch(`${API_BASE}${path}`, opts);
    
    if (!res.ok) {
      const err: ApiError = new Error(`API error: ${res.status} ${res.statusText}`);
      err.status = res.status;
      throw err;
    }

    const ct = res.headers.get('content-type') || '';
    if (ct.includes('application/json')) {
      return res.json();
    }
    return res.text();
  }

  watch<T>(path: string, onMessage: (data: T) => void): () => void {
    const url = `${API_BASE}${path}/watch`;
    const evt = new EventSource(url);
    evt.onmessage = (e) => {
      try {
        onMessage(JSON.parse(e.data));
      } catch {
        // Ignore parse errors
      }
    };
    return () => evt.close();
  }
}

export const client = new TcsClient();
