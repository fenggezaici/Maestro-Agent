/// Desktop E2E over W3C WebDriver (tauri-driver + msedgedriver), using plain
/// fetch so no extra dependency is needed. Covers what preview cannot: the real
/// exe, real IPC and an enabled composer.
///
/// Prerequisites: `cargo build --features tauri/test`, `cargo install tauri-driver`,
/// and `node scripts/e2e-driver-setup.mjs` (matching msedgedriver).
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const exe = path.join(root, 'target', 'debug', 'maestro.exe');
const nativeDriver = path.join(root, 'scripts', 'msedgedriver.exe');
const output = path.join(root, 'artifacts', 'e2e');
const base = 'http://127.0.0.1:4444';
const failures = [];

if (!existsSync(exe)) {
  console.error(`missing ${exe} — run: cargo build --features tauri/test (in src-tauri)`);
  process.exit(1);
}

const driverBinary = existsSync(path.join(os.homedir(), '.cargo', 'bin', 'tauri-driver.exe'))
  ? path.join(os.homedir(), '.cargo', 'bin', 'tauri-driver.exe')
  : 'tauri-driver';
const args = ['--port', '4444'];
if (existsSync(nativeDriver)) args.push('--native-driver', nativeDriver);

// A debug build loads `build.devUrl` (127.0.0.1:1420), so the E2E owns a Vite
// dev server for the run instead of requiring a slow release build.
const vite = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js')], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
let viteLogs = '';
vite.stdout.on('data', (data) => { viteLogs = (viteLogs + data).slice(-2000); });
vite.stderr.on('data', (data) => { viteLogs = (viteLogs + data).slice(-2000); });

async function waitForVite() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch('http://127.0.0.1:1420', { signal: AbortSignal.timeout(1000) });
      if (response.ok) return;
    } catch { /* not up yet */ }
    if (vite.exitCode !== null) throw new Error(`vite exited: ${viteLogs}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`vite did not start: ${viteLogs}`);
}

const driver = spawn(driverBinary, args, { stdio: ['ignore', 'pipe', 'pipe'] });
let driverLogs = '';
driver.stdout.on('data', (data) => { driverLogs = (driverLogs + data).slice(-4000); });
driver.stderr.on('data', (data) => { driverLogs = (driverLogs + data).slice(-4000); });

async function waitForDriver() {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${base}/status`, { signal: AbortSignal.timeout(1000) });
      if (response.ok) return;
    } catch { /* not up yet */ }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`tauri-driver did not start: ${driverLogs}`);
}

let session;
async function wd(method, suffix, body) {
  const response = await fetch(`${base}${suffix}`, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(`${method} ${suffix} → ${response.status} ${JSON.stringify(payload.value ?? payload)}`);
  return payload.value;
}

async function execute(script, scriptArgs = []) {
  return wd('POST', `/session/${session}/execute/sync`, { script, args: scriptArgs });
}

async function check(name, run) {
  try {
    await run();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures.push(`${name}: ${error.message}`);
    console.log(`  FAIL ${name}: ${error.message}`);
  }
}

try {
  await waitForVite();
  await waitForDriver();
  const created = await wd('POST', '/session', {
    capabilities: { alwaysMatch: { browserName: 'wry', 'tauri:options': { application: exe } } },
  });
  session = created.sessionId ?? created['sessionId'];
  await mkdir(output, { recursive: true });

  // The window exists before the embedded UI finishes mounting (Monaco preload,
  // kernel snapshot). Poll instead of guessing a fixed delay.
  async function waitForShell() {
    const deadline = Date.now() + 25_000;
    let last = '';
    while (Date.now() < deadline) {
      try {
        const state = await execute(`return { ready: document.readyState, len: document.body ? document.body.innerHTML.length : -1, hasWorkbench: Boolean(document.querySelector('.workbench')), text: document.body ? document.body.innerText.replace(/\s+/g, ' ').slice(0, 200) : '' };`);
        if (state?.hasWorkbench) return;
        last = JSON.stringify(state);
      } catch (error) { last = error.message; }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error(`shell not found; last=${last}`);
  }

  await check('shell mounts within 25s', waitForShell);

  await check('desktop shell renders', async () => {
    const found = await execute('return Boolean(document.querySelector(".workbench"))');
    if (!found) throw new Error('no .workbench element');
  });

  await check('composer is enabled with real IPC', async () => {
    const disabled = await execute('const el = document.querySelector(\'textarea[aria-label="发送消息"]\'); return el ? el.disabled : null;');
    if (disabled !== false) throw new Error(`composer disabled=${JSON.stringify(disabled)}`);
  });

  await check('mode can be switched to code', async () => {
    await execute(`
      const trigger = [...document.querySelectorAll('button')].find((button) => button.getAttribute('aria-label') === '由 Rust 强制执行');
      trigger?.click();
      return Boolean(trigger);
    `);
    await new Promise((resolve) => setTimeout(resolve, 300));
    const switched = await execute(`
      const option = [...document.querySelectorAll('[role="menuitemradio"], [role="option"], button')]
        .find((item) => item.textContent?.trim().startsWith('代码'));
      option?.click();
      return Boolean(option);
    `);
    if (!switched) throw new Error('mode option not found');
    await new Promise((resolve) => setTimeout(resolve, 500));
    const label = await execute(`
      const trigger = [...document.querySelectorAll('button')].find((button) => button.getAttribute('aria-label') === '由 Rust 强制执行');
      return trigger?.textContent ?? '';
    `);
    if (!label.includes('代码')) throw new Error(`mode label is "${label}"`);
  });

  await check('splitter reports its width over ARIA', async () => {
    const value = await execute(`
      const separator = document.querySelector('[role="separator"][aria-label="调整侧栏宽度"]');
      return separator ? Number(separator.getAttribute('aria-valuenow')) : null;
    `);
    if (!Number.isFinite(value) || value <= 0) throw new Error(`aria-valuenow=${value}`);
  });
} catch (error) {
  failures.push(`session: ${error.message}`);
  console.error(`  FAIL session: ${error.message}\n${driverLogs}`);
} finally {
  if (session) { try { await wd('DELETE', `/session/${session}`); } catch { /* already gone */ } }
  driver.kill();
  vite.kill();
}

if (failures.length) {
  console.error(`\n${failures.length} desktop E2E check(s) failed`);
  process.exit(1);
}
console.log('\ndesktop E2E passed');
