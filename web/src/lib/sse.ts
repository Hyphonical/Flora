// EventSource('/api/events') lifecycle & automatic reconnect/resync client.

import type { ServerSseEvent } from './types';

export interface SseHandlers {
  onReady: () => void;
  onDisconnected: () => void;
  onEvent: (event: ServerSseEvent) => void;
}

export class FloraSseClient {
  private es: EventSource | null = null;
  private url = '/api/events';
  private paused = false;
  private handlers: SseHandlers;

  constructor(handlers: SseHandlers) {
    this.handlers = handlers;
  }

  connect(opts?: { all?: boolean; owner?: string }) {
    this.paused = false;
    const params = new URLSearchParams();
    if (opts?.all) params.set('all', '1');
    else if (opts?.owner) params.set('owner', opts.owner);
    const qs = params.toString();
    const nextUrl = qs ? `/api/events?${qs}` : '/api/events';

    if (this.es && this.url === nextUrl && this.es.readyState !== EventSource.CLOSED) {
      return;
    }

    this.closeInternal();
    this.url = nextUrl;
    this.openSource();
  }

  pause() {
    this.paused = true;
    this.closeInternal();
    this.handlers.onDisconnected();
  }

  resume() {
    if (!this.paused && this.es && this.es.readyState !== EventSource.CLOSED) {
      return;
    }
    this.paused = false;
    this.closeInternal();
    this.openSource();
  }

  close() {
    this.paused = true;
    this.closeInternal();
    this.handlers.onDisconnected();
  }

  get isPaused(): boolean {
    return this.paused;
  }

  private openSource() {
    if (typeof window === 'undefined' || typeof EventSource === 'undefined') return;
    const es = new EventSource(this.url, { withCredentials: true });
    this.es = es;

    es.addEventListener('ready', () => {
      if (this.es === es) {
        this.handlers.onReady();
      }
    });

    es.onmessage = (e: MessageEvent<string>) => {
      if (this.es !== es || !e.data) return;
      try {
        const parsed = JSON.parse(e.data) as ServerSseEvent;
        this.handlers.onEvent(parsed);
      } catch {
        // Ignore malformed frame
      }
    };

    es.onerror = () => {
      if (this.es === es) {
        this.handlers.onDisconnected();
      }
    };
  }

  private closeInternal() {
    if (this.es) {
      this.es.close();
      this.es = null;
    }
  }
}
