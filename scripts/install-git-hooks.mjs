/// Opt-in pre-push hook: run the fast local gate before pushing.
/// Remove with `rm .git/hooks/pre-push`; skip one push with `git push --no-verify`.
import { existsSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const hooks = path.join(root, '.git', 'hooks');
if (!existsSync(hooks)) {
  console.error('未找到 .git/hooks —— 请在仓库根目录执行');
  process.exit(1);
}

const hook = path.join(hooks, 'pre-push');
const body = `#!/bin/sh
# Installed by scripts/install-git-hooks.mjs. Remove: rm .git/hooks/pre-push
echo "[pre-push] 本地 CI（fast）…"
corepack pnpm verify:fast || {
  echo "[pre-push] 本地 CI 未通过，已阻止推送。临时跳过：git push --no-verify"
  exit 1
}
`;
writeFileSync(hook, body, { mode: 0o755 });
console.log(`已安装 ${hook}\n卸载：rm .git/hooks/pre-push`);
