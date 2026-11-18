/// Local CI: the same gates as `.github/workflows/ci.yml`, runnable without
/// GitHub Actions. It bootstraps the MSVC environment for every cargo step, so
/// `cargo` works from any shell (in Git Bash a stray `link.exe` otherwise wins
/// over MSVC's linker and the build fails with a confusing error).
///
/// Usage:
///   corepack pnpm ci            # full local gate
///   corepack pnpm ci:fast       # frontend tests + build only
///   corepack pnpm ci:desktop    # full gate + real-IPC desktop E2E
///   node scripts/ci.mjs --no-e2e --fail-fast
import { execFileSync, spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const flags = new Set(process.argv.slice(2));
const fast = flags.has('--fast');
const noE2e = flags.has('--no-e2e');
const desktop = flags.has('--desktop');
const failFast = flags.has('--fail-fast');

const VSWHERE = 'C:\\Program Files (x86)\\Microsoft Visual Studio\\Installer\\vswhere.exe';

function findVcvars() {
  if (process.platform !== 'win32' || !existsSync(VSWHERE)) return null;
  try {
    const install = execFileSync(VSWHERE, ['-latest', '-products', '*', '-property', 'installationPath'], {
      encoding: 'utf8',
    }).trim();
    if (!install) return null;
    const bat = path.join(install, 'VC', 'Auxiliary', 'Build', 'vcvars64.bat');
    return existsSync(bat) ? bat : null;
  } catch {
    return null;
  }
}

function hasDesktopDriver() {
  const local = path.join(os.homedir(), '.cargo', 'bin', 'tauri-driver.exe');
  return existsSync(local) || existsSync(path.join(root, 'scripts', 'msedgedriver.exe'));
}

const vcvars = findVcvars();
if (!vcvars && process.platform === 'win32') {
  console.warn('[ci] 未找到 vcvars64.bat（VS 2022 Build Tools？）—— cargo 步骤可能因找不到 cl.exe 失败');
}

const steps = [
  { name: 'frontend tests', command: 'corepack pnpm test' },
  { name: 'frontend build', command: 'corepack pnpm build' },
];

if (!fast) {
  steps.push({
    name: 'rust clippy',
    command: 'cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings',
    msvc: true,
  });
  steps.push({
    name: 'rust tests',
    command: 'cargo test --manifest-path src-tauri/Cargo.toml --workspace',
    msvc: true,
  });
  if (!noE2e) steps.push({ name: 'preview e2e', command: 'node scripts/e2e-preview.mjs' });
  if (desktop) {
    if (hasDesktopDriver()) steps.push({ name: 'desktop e2e', command: 'node scripts/e2e-desktop.mjs' });
    else console.warn('[ci] 跳过 desktop e2e：缺少 tauri-driver 或 msedgedriver（node scripts/e2e-driver-setup.mjs）');
  }
}

function run(step) {
  const command = step.msvc && vcvars ? `"${vcvars}" >nul && ${step.command}` : step.command;
  return new Promise((resolve) => {
    const child = spawn(command, { cwd: root, shell: true, stdio: 'inherit', env: process.env });
    child.on('exit', (code) => resolve(code === 0));
    child.on('error', () => resolve(false));
  });
}

console.log(`[ci] ${steps.length} 个步骤${fast ? '（fast）' : ''}${desktop ? '（含桌面 E2E）' : ''}\n`);
const results = [];
let failed = 0;
for (const step of steps) {
  const started = Date.now();
  process.stdout.write(`\n[ci] ▶ ${step.name}\n     $ ${step.command}\n`);
  const ok = await run(step);
  const seconds = ((Date.now() - started) / 1000).toFixed(1);
  results.push({ name: step.name, ok, seconds });
  if (!ok) {
    failed += 1;
    if (failFast) {
      console.log(`[ci] ✗ ${step.name} 失败（${seconds}s），已按 --fail-fast 停止`);
      break;
    }
  }
}

const total = results.reduce((sum, item) => sum + Number(item.seconds), 0).toFixed(1);
console.log('\n[ci] 汇总');
for (const item of results) {
  console.log(`  ${item.ok ? '✓' : '✗'} ${item.name.padEnd(16)} ${item.seconds.padStart(6)}s`);
}
console.log(`[ci] 总耗时 ${total}s`);

if (failed > 0) {
  console.error(`\n[ci] ${failed} 个步骤失败。单独复现：见上方每个步骤的命令。`);
  process.exit(1);
}
console.log('\n[ci] 全部通过');
