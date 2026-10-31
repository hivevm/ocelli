// Verifies: ADR-0035
//
// The remote server's transport carries the desktop host's commands and events: a command goes
// out with an id and its arguments, its answer resolves the call with that id, a refusal rejects
// it with the host's reason, and an event reaches its listeners with the payload the host sent.

import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import { CLOSED, SocketTransport, socketUrl, type SocketLike } from './transport.ts';
import type { RepoWritten } from './types.ts';

/** A socket that records what is sent and lets the test play the server. */
class FakeSocket implements SocketLike {
  readyState = 0;
  sent: { id: number; cmd: string; args: Record<string, unknown> }[] = [];
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onclose: ((ev: CloseEvent) => unknown) | null = null;

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.({} as Event);
  }

  answer(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) } as MessageEvent);
  }

  close(): void {
    this.readyState = 3;
    this.onclose?.({} as CloseEvent);
  }
}

function connected(): { socket: FakeSocket; transport: SocketTransport } {
  const socket = new FakeSocket();
  const transport = new SocketTransport(socket);
  socket.open();
  return { socket, transport };
}

describe('the WebSocket transport', () => {
  test('a command goes out with its arguments and its answer resolves it by id', async () => {
    const { socket, transport } = connected();
    const first = transport.invoke<number[]>('rows_of', { shas: ['a'] });
    const second = transport.invoke<string>('get_file_diff', { sha: 'a', filePath: 'f' });
    expect(socket.sent).toEqual([
      { id: 1, cmd: 'rows_of', args: { shas: ['a'] } },
      { id: 2, cmd: 'get_file_diff', args: { sha: 'a', filePath: 'f' } },
    ]);
    // Answered out of order, each reaches its own call.
    socket.answer({ id: 2, ok: '[]' });
    socket.answer({ id: 1, ok: [7] });
    expect(await first).toEqual([7]);
    expect(await second).toBe('[]');
  });

  test('a command without arguments sends an empty set, and a null answer resolves as null', async () => {
    const { socket, transport } = connected();
    const answer = transport.invoke('startup_repo');
    expect(socket.sent).toEqual([{ id: 1, cmd: 'startup_repo', args: {} }]);
    socket.answer({ id: 1, ok: null });
    expect(await answer).toBeNull();
  });

  test('a refusal rejects the call with the host\'s reason', async () => {
    const { socket, transport } = connected();
    const answer = transport.invoke('open_repo', { path: '../x' });
    socket.answer({ id: 1, err: '../x is no repository below the server root' });
    await expect(answer).rejects.toBe('../x is no repository below the server root');
  });

  test('a command asked before the socket opens goes out once it does', async () => {
    const socket = new FakeSocket();
    const transport = new SocketTransport(socket);
    const answer = transport.invoke('selection');
    expect(socket.sent).toEqual([]);
    socket.open();
    expect(socket.sent).toEqual([{ id: 1, cmd: 'selection', args: {} }]);
    socket.answer({ id: 1, ok: null });
    expect(await answer).toBeNull();
  });

  test('an event reaches its listeners with the payload the host sent, until they stop', async () => {
    // The payload the desktop host sends with a selection, as the core's fixture holds it.
    const fixture = join(import.meta.dir, '..', '..', 'crates', 'core', 'tests', 'fixtures', 'offers.json');
    const payload = await Bun.file(fixture).json();
    const { socket, transport } = connected();
    const got: unknown[] = [];
    const other: unknown[] = [];
    const stop = await transport.listen('load-refs', (e) => got.push(e.payload));
    await transport.listen('set-status', (e) => other.push(e.payload));
    socket.answer({ event: 'load-refs', payload });
    expect(got).toEqual([payload]);
    expect(other).toEqual([]);
    stop();
    socket.answer({ event: 'load-refs', payload });
    expect(got).toHaveLength(1);
  });

  test('the server\'s own repo-written event reaches its listener with the payload its type names', async () => {
    // The message a tab receives once another tab wrote to its repository, as the server's
    // two-tab test writes it (ADR-0035).
    const fixture = join(import.meta.dir, '..', '..', 'crates', 'server', 'tests', 'fixtures', 'repo-written.json');
    const message = await Bun.file(fixture).json();
    expect(message.event).toBe('repo-written');
    const { socket, transport } = connected();
    const got: RepoWritten[] = [];
    await transport.listen<RepoWritten>('repo-written', (e) => got.push(e.payload));
    socket.answer(message);
    expect(got).toEqual([message.payload]);
    expect(typeof got[0]).toBe('string');
  });

  test('a closed connection rejects what waits and what is asked after, and says so', async () => {
    const { socket, transport } = connected();
    const status: unknown[] = [];
    await transport.listen('set-status', (e) => status.push(e.payload));
    const waiting = transport.invoke('read_window', { top: 0, rows: 10 });
    socket.close();
    await expect(waiting).rejects.toBe(CLOSED);
    await expect(transport.invoke('selection')).rejects.toBe(CLOSED);
    expect(status).toEqual([CLOSED]);
  });
});

describe('the address of the socket', () => {
  test('is the page\'s own origin, over TLS where the page came over TLS', () => {
    expect(socketUrl({ protocol: 'http:', host: 'localhost:8000' })).toBe('ws://localhost:8000/ws');
    expect(socketUrl({ protocol: 'https:', host: 'box:9' })).toBe('wss://box:9/ws');
  });
});
