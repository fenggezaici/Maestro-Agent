# Maestro Agent 工作规范

## UI/布局

- **参考Zcode对应UI和布局**：所有 UI 设计、组件布局与交互模式以 ZCode（zai-org/ZCode）对应模块为准。
- ZCode 参考克隆位于仓库外 `D:\zcode-ref`（浅克隆）；改 UI 前先对照其 `packages/ui` 下同名组件（CommandCenter、chat-input-toolbar、ToolCallBlocks、WorkspaceSidebar 等）。
- 不引入 ZCode 依赖本体；参考其布局与交互，用本项目现有 Radix/Tailwind 实现。

## 工作流

- **先写计划，经我同意后方可更改文件**：任何代码/文件修改前先给出计划（写进 plan 文件或直接列出），得到我明确同意后再动手实现。

## 边界

- Preview（`?preview=1`）不调用 `invoke/listen`；live 专属能力（原生对话框、IPC）在 preview 下禁用，不造假。
- Explorer 只读；工具执行、文件写入、命令执行未开放，不伪造后端能力。
- 未知模型的 context/thinking 不编造数字，显示"服务端配置"。

## 构建/运行环境

- 前端包管理：`pnpm` 不在 PATH，用 `corepack pnpm <cmd>`；或一次性 `corepack enable` 把 shim 写进 PATH。
- Rust 构建：Git Bash 的 `link.exe` 会抢先 MSVC linker——必须在 VS DevShell / `vcvars64.bat` 环境跑 `cargo`，或 PowerShell `Launch-VsDevShell.ps1`。
- Tauri 前端内嵌：`frontendDist: "../dist"` 在编译期打进 `maestro.exe`——改了 `src/` 前端后必须 `pnpm build` 再 `cargo build`，否则 exe 里是旧 UI。
- 开发热更：`corepack pnpm tauri dev`（自动起 vite dev server，前端改动即时生效、无需重编 Rust）。
- 冒烟验证：`corepack pnpm build` + `cargo build`（src-tauri）→ `target/debug/maestro.exe`。
