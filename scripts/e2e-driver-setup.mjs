/// Download the msedgedriver build that matches the installed WebView2 runtime.
/// tauri-driver refuses to start when the driver and runtime versions differ,
/// so this keeps `e2e-desktop.mjs` reproducible on a dev box and in CI.
import { execFileSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { mkdir, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';

const root = fileURLToPath(new URL('../', import.meta.url));
const target = path.join(root, 'scripts', 'msedgedriver.exe');

function webview2Version() {
  const keys = [
    'HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
  ];
  for (const key of keys) {
    try {
      const out = execFileSync('reg', ['query', key, '/v', 'pv'], { encoding: 'utf8' });
      const match = /pv\s+REG_SZ\s+([\d.]+)/.exec(out);
      if (match) return match[1].trim();
    } catch { /* try the next hive */ }
  }
  throw new Error('WebView2 runtime not found (install the Evergreen runtime first)');
}

const version = webview2Version();
console.log(`WebView2 runtime: ${version}`);

const url = `https://msedgedriver.microsoft.com/${version}/edgedriver_win64.zip`;
const zip = path.join(root, 'scripts', 'msedgedriver.zip');
await mkdir(path.join(root, 'scripts'), { recursive: true });
const response = await fetch(url);
if (!response.ok) throw new Error(`download failed ${response.status}: ${url}`);
await pipeline(response.body, createWriteStream(zip));
execFileSync('powershell', ['-NoProfile', '-Command', `Expand-Archive -LiteralPath '${zip}' -DestinationPath '${path.join(root, 'scripts')}' -Force`]);
await rm(zip, { force: true });
console.log(`msedgedriver ready: ${target}`);
