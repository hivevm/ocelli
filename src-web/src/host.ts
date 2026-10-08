// The host the web interface runs against, picked once at start-up (ADR-0037): the desktop host
// inside Tauri's WebView, the remote server's WebSocket in a browser.

import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import { SocketTransport, socketUrl, type Transport } from './transport.ts';

const tauri: Transport = {
  invoke: (cmd, args) => tauriInvoke(cmd, args),
  listen: (event, handler) => tauriListen(event, handler),
};

const transport: Transport = isTauri()
  ? tauri
  : new SocketTransport(new WebSocket(socketUrl(window.location)));

export const invoke: Transport['invoke'] = (cmd, args) => transport.invoke(cmd, args);

export const listen: Transport['listen'] = (event, handler) => transport.listen(event, handler);
