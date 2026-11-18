/// Preview E2E: drives the built UI (`?preview=1`) in Chromium and asserts the
/// shell, composer, mode picker, slash panel and the splitter's keyboard
/// contract. Preview never calls IPC, so this covers layout/interaction only —
/// real backend flows live in `e2e-desktop.mjs`.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = fileURLToPath(new URL('../', import.meta.url));
const output = path.join(root, 'artifacts', 'e2e');
const failures = [];

async function freePort(from, to) {
  for (let port = from; port < to; port++) {
    const listener = net.createServer();
    try {
      await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(port, '127.0.0.1', resolve); });
      await new Promise((resolve) => listener.close(resolve));
      return port;
    } catch (error) {
      if (error.code !== 'EADDRINUSE') throw error;
    }
  }
  throw new Error('no free port');
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

const port = await freePort(1470, 1500);
const origin = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), 'preview', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
let logs = '';
server.stdout.on('data', (data) => { logs = (logs + data).slice(-4000); });
server.stderr.on('data', (data) => { logs = (logs + data).slice(-4000); });

let browser;
try {
  const deadline = Date.now() + 30_000;
  while (true) {
    if (server.exitCode !== null) throw new Error(`preview server exited: ${logs}`);
    try { if ((await fetch(origin, { signal: AbortSignal.timeout(1000) })).ok) break; } catch { /* retry */ }
    if (Date.now() > deadline) throw new Error(`preview server timed out: ${logs}`);
    await new Promise((resolve) => setTimeout(resolve, 150));
  }

  await mkdir(output, { recursive: true });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1480, height: 920 }, reducedMotion: 'reduce' });
  const consoleErrors = [];
  page.on('pageerror', (error) => consoleErrors.push(error.message));
  await page.goto(`${origin}/?preview=1`, { waitUntil: 'networkidle' });

  await check('shell renders with the preview tag', async () => {
    await page.waitForSelector('.workbench', { timeout: 15_000 });
    assert.equal(await page.locator('.preview-tag').count(), 1);
  });

  await check('composer is present but disabled in preview (no IPC)', async () => {
    const input = page.getByLabel('发送消息');
    await input.waitFor({ timeout: 5_000 });
    assert.equal(await input.isDisabled(), true);
  });

  await check('mode picker lists every mode', async () => {
    await page.getByLabel('由 Rust 强制执行').click();
    for (const label of ['计划', '代码', '确认', '自动']) {
      await page.getByText(label, { exact: true }).first().waitFor({ timeout: 5_000 });
    }
    await page.keyboard.press('Escape');
  });

  await check('sidebar splitter is keyboard operable', async () => {
    const separator = page.getByRole('separator', { name: '调整侧栏宽度' });
    const before = Number(await separator.getAttribute('aria-valuenow'));
    await separator.focus();
    await page.keyboard.press('ArrowRight');
    const after = Number(await separator.getAttribute('aria-valuenow'));
    assert.ok(after > before, `expected ${after} > ${before}`);
    await page.keyboard.press('Home');
    assert.equal(Number(await separator.getAttribute('aria-valuenow')), 220);
  });

  await check('project editor dialog renames a project', async () => {
    const row = page.locator('.project-row').first();
    const name = (await row.locator('.project-name').textContent()).trim();
    await row.getByLabel(new RegExp(`项目 .* 操作`)).click();
    await page.getByText('编辑项目', { exact: true }).click();
    const input = page.getByLabel('项目名称');
    await input.waitFor({ timeout: 5_000 });
    assert.equal(await input.inputValue(), name);
    await input.fill(`${name}-改`);
    await page.getByRole('button', { name: '保存' }).click();
    await page.locator('.project-name', { hasText: `${name}-改` }).first().waitFor({ timeout: 5_000 });
  });

  await check('no uncaught page errors', async () => {
    assert.deepEqual(consoleErrors, []);
  });

  await page.screenshot({ path: path.join(output, 'preview-shell.png'), fullPage: false });
} finally {
  await browser?.close();
  server.kill();
}

if (failures.length) {
  console.error(`\n${failures.length} preview E2E check(s) failed`);
  process.exit(1);
}
console.log('\npreview E2E passed');
