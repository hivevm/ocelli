// Verifies: ADR-0023, ADR-0008
//
// Two decisions that live in configuration rather than in code, where a regression is silent:
// the WebView's granted surface (ADR-0023) and the single package manager (ADR-0008). A wrong
// value here opens egress or lets a second lockfile decide what gets installed, and nothing in
// the running application says so. These tests read the files that ship.

import { describe, expect, it } from 'bun:test';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dir, '..', '..');
const conf = await Bun.file(join(ROOT, 'src-tauri', 'tauri.conf.json')).json();
const security = conf.app?.security ?? {};

describe('WebView surface (ADR-0023)', () => {
  it('sets a content security policy instead of leaving it open', () => {
    expect(typeof security.csp).toBe('string');
    expect(security.csp).toContain("default-src 'self'");
    expect(security.csp).toContain("script-src 'self'");
  });

  it('grants exactly one third-party origin, and only for images', () => {
    // 'http://ipc.localhost' is Tauri's own IPC transport, not a third party: the frontend
    // reaches the Rust side through it. Everything else with a scheme is somebody else's host.
    const IPC_ORIGIN = 'http://ipc.localhost';
    const directives = (security.csp as string).split(';').map((d) => d.trim()).filter(Boolean);
    const external: string[] = [];
    for (const directive of directives) {
      const [name, ...values] = directive.split(/\s+/);
      for (const value of values) {
        if (/^https?:\/\//.test(value) && value !== IPC_ORIGIN) external.push(`${name} ${value}`);
      }
    }
    expect(external).toEqual(['img-src https://www.gravatar.com']);
  });

  it('leaves the asset protocol disabled', () => {
    expect(security.assetProtocol?.enable ?? false).toBe(false);
    expect(security.assetProtocol?.scope ?? []).not.toContain('**');
  });

  it('keeps the global Tauri object out of the window', () => {
    expect(conf.app?.withGlobalTauri ?? false).toBe(false);
  });
});

describe('one package manager (ADR-0008)', () => {
  it('ships bun.lock', () => {
    expect(existsSync(join(ROOT, 'bun.lock'))).toBe(true);
  });

  it('ships no lockfile of another package manager', () => {
    for (const other of ['package-lock.json', 'yarn.lock', 'pnpm-lock.yaml', 'npm-shrinkwrap.json']) {
      expect(existsSync(join(ROOT, other)), other).toBe(false);
    }
  });
});
