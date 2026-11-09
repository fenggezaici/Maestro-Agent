import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = fileURLToPath(new URL('../', import.meta.url));
const output = path.join(root, 'artifacts', 'ui');
// OS-assigned ports can land on browser-blocked ports such as 10080.
let port;
for (let candidate = 1430; candidate < 1460; candidate++) {
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
server.stdout.on('data', (data) => { logs = (logs + data).slice(-8000); });
server.stderr.on('data', (data) => { logs = (logs + data).slice(-8000); });
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
  const errors = [];
  const external = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => { if (/^https?:/.test(request.url()) && !request.url().startsWith(`${origin}/`)) external.push(request.url()); });
  // Exercise the same production CSP as the desktop frontend, without altering Rust configuration.
  const config = JSON.parse(await readFile(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  await page.route('**/*', async (route) => {
    if (route.request().isNavigationRequest()) {
      const response = await route.fetch();
      await route.fulfill({ response, headers: { ...response.headers(), 'content-security-policy': config.app.security.csp } });
    } else await route.continue();
  });
  await page.goto(`${origin}/?preview=1`, { waitUntil: 'networkidle' });
  await page.locator('[data-editor-ready="true"]').waitFor({ timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  assert(await page.getByText('Frontend preview', { exact: true }).isVisible());
  assert(await page.getByRole('button', { name: '新建会话', exact: true }).isEnabled());
  assert(await page.getByRole('button', { name: '批准计划' }).isDisabled());
  assert(await page.getByRole('button', { name: '发送', exact: true }).isDisabled());
  const layout = await page.evaluate(() => ({
    sidebar: document.querySelector('.sessions-pane').getBoundingClientRect().width,
    diff: document.querySelector('.review-pane').getBoundingClientRect().width,
    width: innerWidth,
    horizontalOverflow: document.documentElement.scrollWidth > innerWidth,
    background: getComputedStyle(document.querySelector('.conversation-pane')).backgroundColor,
  }));
  assert(Math.abs(layout.sidebar - 240) < 2);
  assert(Math.abs(layout.diff / layout.width - 0.45) < 0.01);
  assert.equal(layout.background, 'rgb(23, 23, 23)');
  assert.equal(layout.horizontalOverflow, false);
  const visibleApproval = await page.getByRole('button', { name: '批准计划' }).boundingBox();
  const composer = await page.locator('.composer-area').boundingBox();
  assert(visibleApproval && composer && visibleApproval.y + visibleApproval.height <= composer.y, 'The approval action must be visible above the composer at the reference viewport.');
  await page.screenshot({ path: path.join(output, 'maestro-workbench.png'), animations: 'disabled' });
  assert((await page.getByLabel('由 Rust 强制执行', { exact: true }).textContent()).includes('Code'));
  assert((await page.getByLabel('选择模型', { exact: true }).textContent()).includes('deepseek-chat'));
  await page.screenshot({ path: path.join(output, 'maestro-composer-controls.png'), animations: 'disabled' });
  assert.equal(await page.getByRole('button', { name: '模型参数', exact: true }).count(), 0);
  const row = await page.locator('.composer-toolbar').evaluate((toolbar) => {
    const nodes = [...toolbar.querySelectorAll('[role="combobox"], .context-meter')];
    return nodes.map((node) => { const bounds = node.getBoundingClientRect(); return bounds.y + bounds.height / 2; });
  });
  assert(Math.max(...row) - Math.min(...row) < 3, 'Composer choices and Tokens must be laid out in one row.');
  await page.getByLabel('选择模型', { exact: true }).click();
  await page.getByRole('option', { name: 'gpt-5.2' }).click();
  const contextChoice = page.getByLabel('思考强度', { exact: true });
  await contextChoice.click();
  const contextOption = page.getByRole('option', { name: 'High', exact: true });
  await contextOption.waitFor();
  const optionBox = await contextOption.boundingBox();
  assert(optionBox);
  const point = { x: optionBox.x + optionBox.width / 2, y: optionBox.y + optionBox.height / 2 };
  assert(await contextOption.evaluate((element, point) => document.elementFromPoint(point.x, point.y)?.closest('[role="option"]') === element, point), 'Flat context choice must remain clickable.');
  await page.screenshot({ path: path.join(output, 'maestro-context-dropdown.png'), animations: 'disabled' });
  await contextOption.click();
  assert((await contextChoice.textContent()).includes('High'));
  assert(await page.getByText('400k context', { exact: true }).isVisible());
  assert((await page.getByLabel('实时会话统计', { exact: true }).textContent()).includes('Turns: 1'));
  assert.equal(await page.getByRole('listbox').count(), 0);
  const footer = await page.locator('.sidebar-bottom').boundingBox();
  const sidebar = await page.locator('.sessions-pane').boundingBox();
  const settingsButton = await page.getByRole('button', { name: 'Settings', exact: true }).boundingBox();
  assert(footer && sidebar && settingsButton && settingsButton.y >= footer.y && Math.abs(footer.y + footer.height - sidebar.y - sidebar.height) < 2);
  await page.locator('.project-tree').evaluate((tree) => { const spacer = document.createElement('div'); spacer.dataset.footerScrollTest = 'true'; spacer.style.height = '1200px'; tree.append(spacer); tree.scrollTop = tree.scrollHeight; });
  await settle();
  assert(Math.abs((await page.locator('.sidebar-bottom').boundingBox()).y - footer.y) < 2, 'Settings footer must remain fixed while sessions scroll.');
  await page.locator('.project-tree').evaluate((tree) => { tree.querySelector('[data-footer-scroll-test]')?.remove(); tree.scrollTop = 0; });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  assert(await page.getByLabel('主题', { exact: true }).isVisible());
  await page.getByLabel('主题', { exact: true }).click();
  await page.getByRole('option', { name: 'Light', exact: true }).click();
  await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), 'rgb(255, 255, 255)');
  await page.screenshot({ path: path.join(output, 'maestro-light-theme.png'), animations: 'disabled' });
  await page.getByLabel('主题', { exact: true }).click();
  await page.getByRole('option', { name: 'Dark', exact: true }).click();
  await page.screenshot({ path: path.join(output, 'maestro-settings.png'), animations: 'disabled' });
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await page.getByRole('button', { name: 'Anthropic', exact: true }).click();
  assert.equal(await page.getByLabel('Base URL', { exact: true }).inputValue(), 'https://api.anthropic.com/v1');
  assert(await page.getByLabel('API Key', { exact: true }).isDisabled());
  await page.getByRole('button', { name: 'Shortcuts', exact: true }).click();
  await page.getByLabel('搜索快捷键', { exact: true }).fill('Open Activity');
  await page.getByRole('button', { name: '重新绑定 Open Activity', exact: true }).click();
  await page.keyboard.press('Control+Shift+Y');
  assert((await page.getByRole('dialog').textContent()).includes('Ctrl+Shift+Y'));
  await page.getByRole('button', { name: '重置 Open Activity', exact: true }).click();
  await page.getByRole('button', { name: '关闭设置', exact: true }).click();
  await page.getByRole('tab', { name: 'Explorer', exact: true }).click();
  await page.locator('[data-code-ready="true"]').waitFor();
  await page.getByRole('button', { name: /BridgeSession.cpp/ }).click();
  await page.getByLabel('文件预览 src/bridge/BridgeSession.cpp', { exact: true }).waitFor();
  await page.locator('[data-code-ready="true"]').waitFor();
  await page.screenshot({ path: path.join(output, 'maestro-explorer.png'), animations: 'disabled' });
  await page.getByRole('button', { name: '文件夹 bridge', exact: true }).click();
  assert.equal(await page.getByRole('button', { name: '文件夹 bridge', exact: true }).getAttribute('aria-expanded'), 'false');
  await page.getByRole('tab', { name: 'Review', exact: true }).click();
  await page.locator('[data-editor-ready="true"]').waitFor();
  await page.getByRole('button', { name: /^Extensions/ }).click();
  assert(await page.getByRole('switch', { name: '启用 filesystem', exact: true }).isVisible());
  await page.getByRole('switch', { name: '启用 github', exact: true }).click();
  assert.equal(await page.getByRole('switch', { name: '启用 github', exact: true }).getAttribute('data-state'), 'checked');
  await page.getByRole('tab', { name: '发现', exact: true }).click();
  await page.screenshot({ path: path.join(output, 'maestro-extensions.png'), animations: 'disabled' });
  await page.getByRole('button', { name: /^Agents/ }).click();
  async function captureDashboard(name) {
    const reviewWidth = await page.locator('.review-pane').evaluate((element) => element.getBoundingClientRect().width);
    await page.getByRole('button', { name: 'Activity', exact: true }).click();
    await page.getByRole('region', { name: 'Activity Dashboard', exact: true }).waitFor();
    await page.locator('.recharts-line-curve').waitFor();
    assert.equal(await page.getByRole('button', { name: 'Activity', exact: true }).getAttribute('aria-current'), 'page');
    assert.equal(await page.getByRole('dialog').count(), 0);
    assert.equal(await page.getByLabel('发送消息', { exact: true }).count(), 0);
    assert.equal(await page.locator('.metric-card').count(), 4);
    assert(await page.locator('[data-heatmap-day]').count() > 180);
    assert(Math.abs(await page.locator('.review-pane').evaluate((element) => element.getBoundingClientRect().width) - reviewWidth) < 2);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.screenshot({ path: path.join(output, name), animations: 'disabled', fullPage: (page.viewportSize()?.width ?? 0) < 650 });
    await page.getByRole('button', { name: 'Back to Chat', exact: true }).click();
    await page.getByLabel('发送消息', { exact: true }).waitFor();
  }
  await captureDashboard('maestro-dashboard.png');
  await page.getByRole('button', { name: 'Activity', exact: true }).click();
  await page.locator('.recharts-line-curve').waitFor();
  await page.locator('[data-heatmap-day="2026-11-08"]').hover();
  await page.getByRole('tooltip').waitFor();
  assert((await page.getByRole('tooltip').textContent()).includes('修改 120 行代码'));
  await page.screenshot({ path: path.join(output, 'maestro-activity.png'), animations: 'disabled' });
  await page.getByRole('button', { name: /^Agents/ }).click();
  await page.getByLabel('发送消息', { exact: true }).waitFor();

  async function settle() {
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.waitForTimeout(100);
  }
  async function drag(label, dx, dy = 0) {
    const bounds = await page.getByRole('separator', { name: label, exact: true }).boundingBox();
    assert(bounds);
    await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width / 2 + dx, bounds.y + bounds.height / 2 + dy, { steps: 18 });
    await page.mouse.up();
    await settle();
  }
  await drag('调整侧栏宽度', 60);
  const resizedSidebar = await page.locator('.sessions-pane').evaluate((element) => element.getBoundingClientRect().width);
  assert(resizedSidebar > layout.sidebar + 30);
  await drag('调整对话与 Diff 宽度', -70);
  const resizedReview = await page.locator('.review-pane').evaluate((element) => element.getBoundingClientRect().width);
  assert(resizedReview > layout.diff + 30);
  const editorSize = await page.evaluate(() => ({ host: document.querySelector('.monaco-container').getBoundingClientRect().width, editor: document.querySelector('.monaco-diff-editor').getBoundingClientRect().width, overflow: document.documentElement.scrollWidth > innerWidth }));
  assert(Math.abs(editorSize.host - editorSize.editor) < 2);
  assert.equal(editorSize.overflow, false);
  await page.screenshot({ path: path.join(output, 'maestro-workbench-resized.png'), animations: 'disabled' });

  await page.reload({ waitUntil: 'networkidle' });
  await page.locator('[data-editor-ready="true"]').waitFor();
  assert(await page.getByRole('button', { name: /Refactor session lifecycle/ }).isVisible());
  assert(Math.abs(await page.locator('.sessions-pane').evaluate((element) => element.getBoundingClientRect().width) - resizedSidebar) < 2);
  const sidebarHandle = page.getByRole('separator', { name: '调整侧栏宽度', exact: true });
  await sidebarHandle.focus();
  await page.keyboard.press('ArrowLeft');
  await settle();
  assert(await page.locator('.sessions-pane').evaluate((element) => element.getBoundingClientRect().width) < resizedSidebar);
  await page.getByRole('tab', { name: 'tests/BridgeSession.test.cpp', exact: true }).click();
  await page.getByLabel('Diff tests/BridgeSession.test.cpp', { exact: true }).waitFor();
  await page.locator('[data-editor-ready="true"]').waitFor();
  await page.getByRole('tab', { name: '行内差异', exact: true }).click();
  await page.locator('[data-editor-ready="true"]').waitFor();
  await page.getByRole('button', { name: '隐藏代码审查', exact: true }).click();
  assert.equal(await page.getByRole('separator').count(), 1);
  await page.getByRole('button', { name: '显示代码审查', exact: true }).click();
  await page.locator('[data-editor-ready="true"]').waitFor();
  assert.equal(await page.getByRole('tab', { name: '行内差异', exact: true }).getAttribute('data-state'), 'active');
  assert.equal(await page.getByRole('tab', { name: 'tests/BridgeSession.test.cpp', exact: true }).getAttribute('data-state'), 'active');
  await page.getByRole('tab', { name: 'src/bridge/BridgeSession.h', exact: true }).click();
  await page.getByRole('tab', { name: '并排差异', exact: true }).click();
  await page.locator('[data-editor-ready="true"]').waitFor();
  await page.setViewportSize({ width: 1280, height: 900 });
  await settle();
  await page.screenshot({ path: path.join(output, 'maestro-workbench-1280.png'), animations: 'disabled' });
  await captureDashboard('maestro-dashboard-1280.png');
  await page.setViewportSize({ width: 900, height: 900 });
  await page.locator('[data-layout="stacked"]').waitFor();
  await page.locator('[data-editor-ready="true"]').waitFor();
  await settle();
  const narrow = await page.evaluate(() => ({ chat: document.querySelector('.conversation-pane').getBoundingClientRect().bottom, diff: document.querySelector('.review-pane').getBoundingClientRect().top }));
  assert(narrow.diff >= narrow.chat - 1);
  await drag('调整对话与 Diff 高度', 0, -50);
  await page.screenshot({ path: path.join(output, 'maestro-workbench-stacked.png'), animations: 'disabled' });
  await captureDashboard('maestro-dashboard-stacked.png');
  await page.setViewportSize({ width: 640, height: 900 });
  await page.locator('[data-layout="mobile"]').waitFor();
  await page.locator('[data-editor-ready="true"]').waitFor();
  assert.equal(await page.getByRole('separator').count(), 0);
  assert(await page.getByRole('button', { name: 'Settings', exact: true }).isVisible());
  await page.screenshot({ path: path.join(output, 'maestro-workbench-mobile.png'), animations: 'disabled', fullPage: true });
  await captureDashboard('maestro-dashboard-mobile.png');
  // Reproduce the reported model ID without reading or writing live credentials.
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.evaluate(() => {
    const key = 'maestro.ui.v1.preview';
    localStorage.setItem(key, JSON.stringify({ ...JSON.parse(localStorage.getItem(key) ?? '{}'), provider: 'openai', previewModel: 'deepseek-r1', theme: 'light', language: 'zh-CN' }));
  });
  await page.reload({ waitUntil: 'networkidle' });
  await page.locator('[data-editor-ready="true"]').waitFor();
  assert(await page.getByLabel('思考模式：模型内置推理', { exact: true }).isVisible());
  assert((await page.getByLabel('上下文长度', { exact: true }).textContent()).includes('128k context'));
  assert.equal(await page.getByText('Unknown context', { exact: true }).count(), 0);
  const capabilityLayout = await page.locator('.composer-toolbar').evaluate((toolbar) => ({ scroll: toolbar.scrollWidth, width: toolbar.clientWidth }));
  assert(capabilityLayout.scroll <= capabilityLayout.width + 1, 'Capability controls must wrap instead of disappearing into horizontal overflow.');
  await page.locator('.composer-area').screenshot({ path: path.join(output, 'maestro-r1-context.png'), animations: 'disabled' });
  await page.getByLabel('选择模型', { exact: true }).click();
  await page.getByRole('option', { name: 'gpt-5.2' }).click();
  await page.getByLabel('思考强度', { exact: true }).click();
  const high = page.getByRole('option', { name: 'High', exact: true });
  assert(await high.isVisible());
  await page.screenshot({ path: path.join(output, 'maestro-thinking-strength.png'), animations: 'disabled' });
  await high.click();
  assert((await page.getByLabel('思考强度', { exact: true }).textContent()).includes('High'));
  assert((await page.getByLabel('实时会话统计', { exact: true }).textContent()).includes('400.0k'));
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: '模型', exact: true }).click();
  const capabilities = page.getByRole('group', { name: '模型能力', exact: true });
  await page.getByLabel('常用 Model', { exact: true }).click();
  await page.getByRole('option', { name: 'gpt-5.2' }).click();
  await capabilities.getByLabel('思考强度', { exact: true }).click();
  await page.getByRole('option', { name: 'High', exact: true }).click();
  await capabilities.getByText('400k context', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(output, 'maestro-model-capabilities.png'), animations: 'disabled' });
  await page.getByRole('button', { name: 'Ollama', exact: true }).click();
  await page.getByRole('button', { name: '关闭设置', exact: true }).click();
  await page.getByLabel('选择模型', { exact: true }).click();
  await page.getByRole('option', { name: 'deepseek-r1', exact: true }).click();
  assert(await page.getByLabel('思考模式：模型内置推理', { exact: true }).isVisible());
  assert((await page.getByLabel('上下文长度', { exact: true }).getAttribute('title')).includes('num_ctx'));
  assert.deepEqual(errors, []);
  assert.deepEqual(external, []);
  console.log('PASS: R1 context/identity, dynamic thinking and Settings capabilities, non-overflowing composer, fixed Settings footer, readonly preview, Monaco/Review, dashboard routing, resizable panels and browser preview isolation.');
  console.log(`Screenshots: ${output}`);
} finally {
  await browser?.close();
  server.kill();
}
