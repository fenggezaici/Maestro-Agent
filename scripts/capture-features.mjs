// Refresh the README feature shots: workbench, architecture sandbox,
// squad subnet (Houdini), activity dashboard. Assertion-free like
// capture-preview.mjs — capture-ui.mjs remains the strict QA harness.
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
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

  await mkdir(output, { recursive: true });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 1, reducedMotion: 'reduce' });
  // Pin UI scale to 100% like capture-ui.mjs for deterministic captures.
  await page.addInitScript(() => {
    const prev = JSON.parse(localStorage.getItem('maestro.ui.v1.preview') ?? '{}');
    localStorage.setItem('maestro.ui.v1.preview', JSON.stringify({ ...prev, fontScale: '100' }));
  });
  await page.goto(`${origin}/?preview=1`, { waitUntil: 'networkidle' });
  await page.locator('.sessions-pane').waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: /新建会话/ }).first().waitFor({ timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);

  const settle = () => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))).then(() => page.waitForTimeout(150));

  // 1) Workbench main view.
  await settle();
  await page.screenshot({ path: path.join(output, 'maestro-workbench.png'), animations: 'disabled' });
  console.log('captured maestro-workbench.png');

  // 2) Architecture sandbox — preview auto-loads the sample rule graph.
  await page.getByRole('button', { name: /架构沙盘|Architecture Sandbox/ }).first().click();
  await page.locator('.react-flow__node').first().waitFor({ timeout: 15_000 });
  await settle();
  await page.screenshot({ path: path.join(output, 'maestro-sandbox.png'), animations: 'disabled' });
  console.log('captured maestro-sandbox.png');

  // 3) Squad subnet — insert the 调研军团 template, then double-click the
  // squad node (Houdini subnet) to open its inner orchestration canvas.
  await page.getByRole('button', { name: /预置模板/ }).click();
  await page.getByRole('button', { name: /调研军团/ }).click();
  await page.locator('.react-flow__node', { hasText: '双击进入编排' }).waitFor({ timeout: 10_000 });
  await settle();
  await page.locator('.react-flow__node', { hasText: '双击进入编排' }).dblclick();
  await page.locator('.squad-crumb').waitFor({ timeout: 10_000 });
  await settle();
  await page.screenshot({ path: path.join(output, 'maestro-squad.png'), animations: 'disabled' });
  console.log('captured maestro-squad.png');

  // 4) Back to chat → Activity dashboard.
  await page.getByRole('button', { name: /返回对话|Back to Chat|←/ }).first().click();
  await page.locator('[aria-label="活动"]').click();
  await page.getByRole('region', { name: 'Activity Dashboard', exact: true }).waitFor({ timeout: 15_000 });
  await page.locator('.recharts-line-curve').first().waitFor({ timeout: 15_000 });
  await settle();
  await page.screenshot({ path: path.join(output, 'maestro-activity.png'), animations: 'disabled' });
  console.log('captured maestro-activity.png');
} finally {
  await browser?.close();
  server.kill();
}
