// The way the web interface reaches its host (ADR-0035): the desktop host's Tauri commands and
// events, or the remote server's WebSocket, which carries the same commands and events with the
// same names and payloads. The components see neither; `host.ts` picks one at start-up.

/** An event as a listener receives it, the shape Tauri's `listen` hands over. */
export interface HostEvent<T> {
  payload: T;
}

export type Unlisten = () => void;

export interface Transport {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, handler: (e: HostEvent<T>) => void): Promise<Unlisten>;
}

/** What the WebSocket transport needs of a socket; a browser's `WebSocket` has it. */
export interface SocketLike {
  readonly readyState: number;
  send(data: string): void;
  onopen: ((ev: Event) => unknown) | null;
  onmessage: ((ev: MessageEvent) => unknown) | null;
  onclose: ((ev: CloseEvent) => unknown) | null;
}

/** `WebSocket.OPEN`, spelled out so the helper runs where no `WebSocket` exists. */
const OPEN = 1;

/** What the status bar says once the connection is gone: the tab's load session went with it. */
export const CLOSED = 'The connection to ocelli-server closed: reload the page to open the repository again';

/** The address of the server's WebSocket, on the origin the page was served from. */
export function socketUrl(location: { protocol: string; host: string }): string {
  return `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;
}

/**
 * The remote server's transport: a command goes out as `{id, cmd, args}`, its answer comes back
 * as `{id, ok}` or `{id, err}`, and an event arrives as `{event, payload}` (ADR-0035).
 */
export class SocketTransport implements Transport {
  private next = 0;
  private readonly pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: string) => void }>();
  private readonly listeners = new Map<string, Set<(e: HostEvent<unknown>) => void>>();
  /** Commands sent before the socket opened, sent once it does. */
  private queued: string[] = [];
  private closed = false;

  constructor(private readonly socket: SocketLike) {
    socket.onopen = () => {
      for (const text of this.queued) socket.send(text);
      this.queued = [];
    };
    socket.onmessage = (ev) => this.receive(String(ev.data));
    socket.onclose = () => this.close();
  }

  invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    if (this.closed) return Promise.reject(CLOSED);
    const id = ++this.next;
    const text = JSON.stringify({ id, cmd, args });
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      if (this.socket.readyState === OPEN) this.socket.send(text);
      else this.queued.push(text);
    });
  }

  listen<T>(event: string, handler: (e: HostEvent<T>) => void): Promise<Unlisten> {
    const set = this.listeners.get(event) ?? new Set();
    this.listeners.set(event, set);
    const h = handler as (e: HostEvent<unknown>) => void;
    set.add(h);
    return Promise.resolve(() => { set.delete(h); });
  }

  private receive(text: string): void {
    let message: { id?: number; ok?: unknown; err?: string; event?: string; payload?: unknown };
    try {
      message = JSON.parse(text);
    } catch {
      return;
    }
    if (typeof message.id === 'number') {
      const waiting = this.pending.get(message.id);
      if (!waiting) return;
      this.pending.delete(message.id);
      if ('err' in message) waiting.reject(String(message.err));
      else waiting.resolve(message.ok ?? null);
    } else if (typeof message.event === 'string') {
      this.deliver(message.event, message.payload);
    }
  }

  private deliver(event: string, payload: unknown): void {
    for (const handler of this.listeners.get(event) ?? []) handler({ payload });
  }

  /** Every command still waiting is rejected, and the status bar says why. */
  private close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const { reject } of this.pending.values()) reject(CLOSED);
    this.pending.clear();
    this.deliver('set-status', CLOSED);
  }
}
