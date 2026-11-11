// Regenerate the README preview images from the isolated browser preview.
// Kept assertion-free on purpose: capture-ui.mjs is the strict QA harness.
import { spawn } from 'node:child_process';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = fileURLToPath(new URL('../', import.meta.url));
const output = path.join(root, 'artifacts', 'ui');

let port;
for (let candidate = 1470; candidate < 1500; candidate++) {
  const listener = net.createServer();
  try {
    await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(candidate, '127.0.0.1', resolve); });
    await new Promise((resolve) => listener.close(resolve));
    port = candidate;
    break;
  } catch (error) { if (error.code !== 'EADDRINUSE') throw error; }
}
if (!port) throw new Error('No safe local preview port is available.');

const origin = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), 'preview', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
let logs = '';
server.stdout.on('data', (data) => { logs = (logs + data).slice(-4000); });
server.stderr.on('data', (data) => { logs = (logs + data).slice(-4000); });

let browser;
try {
  const deadline = Date.now() + 30_000;
  while (true) {
    if (server.exitCode !== null) throw new Error(`Preview server exited: ${logs}`);
    try { if ((await fetch(origin, { signal: AbortSignal.timeout(1000) })).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error(`Preview server timed out: ${logs}`);
    await new Promise((resolve) => setTimeout(resolve, 150));
  }

  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 1, reducedMotion: 'reduce' });
  await page.goto(`${origin}/?preview=1`, { waitUntil: 'networkidle' });
  await page.getByText('Frontend preview', { exact: true }).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: '切换右侧面板' }).click();
  await page.locator('.right-pane').waitFor({ timeout: 10_000 });
  await page.locator('[data-editor-ready="true"]').waitFor({ timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(output, 'maestro-workbench.png'), animations: 'disabled' });

  await page.getByRole('button', { name: '文件列表视图' }).click();
  await page.getByRole('menuitem', { name: /Explorer/ }).click();
  await page.getByRole('button', { name: '文件 src/bridge/BridgeSession.cpp' }).click();
  await page.locator('[data-code-ready="true"]').waitFor({ timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(output, 'maestro-explorer.png'), animations: 'disabled' });
  console.log('captured maestro-workbench.png and maestro-explorer.png');
} finally {
  await browser?.close();
  server.kill();
}
