# Maestro Agent 工作规范

- 所有面向用户的回复一律使用中文（包括进度说明、诊断结论和最终总结）。
- **相同功能和逻辑可以直接参考 Zcode**：遇到与本项目相同的功能、交互或实现逻辑，直接对照 `D:\zcode-ref` 的对应实现与行为落地（不限于 UI）；仍不引入 ZCode 依赖本体。

## UI/布局

- **参考Zcode对应UI和布局**：所有 UI 设计、组件布局与交互模式以 ZCode（zai-org/ZCode）对应模块为准。
- ZCode 参考克隆位于仓库外 `D:\zcode-ref`（浅克隆）；改 UI 前先对照其 `packages/ui` 下同名组件（CommandCenter、chat-input-toolbar、ToolCallBlocks、WorkspaceSidebar 等）。
- 不引入 ZCode 依赖本体；参考其布局与交互，用本项目现有 Radix/Tailwind 实现。

## 工作流

- **先写计划，经我同意后方可更改文件**：任何代码/文件修改前先给出计划（写进 plan 文件或直接列出），得到我明确同意后再动手实现。
- **git 提交不携带 AI 署名**：提交信息不写 `Generated with …`、`Co-Authored-By: …` 等工具生成尾注。
- **stderr 不留诊断日志**：不在 Rust 侧使用 `eprintln!`/`println!` 输出运行诊断；用户可见错误经 `Err` 上抛到 UI。

## 边界

- Preview（`?preview=1`）不调用 `invoke/listen`；live 专属能力（原生对话框、IPC）在 preview 下禁用，不造假。
- Explorer 只读；Plan 下写入/非白名单命令/非只读 MCP 被物理剥离，白名单只读命令经 run_command 直接执行免审批（严格 argv 解析+无 shell，支持 ≤4 段手工管道；background 不可用）；Code 对齐 ZCode 自主模式（写文件直接执行、命令按风险分类器判定——未命中危险规则直接执行、命中才审批；分类是尽力拦截而非安全边界，嵌套 shell/执行器一律询问），Confirm 全部审批，Auto 直接执行；历史授权与会话放行只免除审批，不能越过当前模式的禁止；不伪造未实现的后端能力。
- 未知模型的 context/thinking 不编造数字，显示"服务端配置"。

## 构建/运行环境

- 前端包管理：`pnpm` 不在 PATH，用 `corepack pnpm <cmd>`；或一次性 `corepack enable` 把 shim 写进 PATH。
- Rust 构建：Git Bash 的 `link.exe` 会抢先 MSVC linker——必须在 VS DevShell / `vcvars64.bat` 环境跑 `cargo`，或 PowerShell `Launch-VsDevShell.ps1`。
- Tauri 前端内嵌：`frontendDist: "../dist"` 在编译期打进 `maestro.exe`——改了 `src/` 前端后必须 `pnpm build` 再 `cargo build`，否则 exe 里是旧 UI。
- 开发热更：`corepack pnpm tauri dev`（自动起 vite dev server，前端改动即时生效、无需重编 Rust）。
- 冒烟验证：`corepack pnpm build` + `cargo build`（src-tauri）→ `target/debug/maestro.exe`。
- **推送远端前门禁：`corepack pnpm verify`**（`scripts/ci.mjs`，与 CI 同构：前端测试 + build + `cargo clippy --all-targets -D warnings` + `cargo test --workspace` + preview e2e）。⚠️ `pnpm ci` 不是门禁——它命中 pnpm 内置的清理安装命令（只装依赖不跑测试），真正的门禁脚本名是 `verify`。只跑 `cargo test`/`check` 会漏 clippy——CI 的 Rust lint 步曾因此连挂多提交。
- 工具链由 `rust-toolchain.toml` 钉在 1.97 minor：升 minor 时改 channel 并顺手清新 clippy lint。
