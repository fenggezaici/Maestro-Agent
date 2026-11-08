# Maestro-Agent

**A Rust-native AI desktop workbench — streaming conversations, persistent sessions, and read-only workspace exploration.**

Maestro-Agent 是基于 **Tauri 2 + React 19 + Rust** 的早期 AI 桌面工作台。它将对话、会话管理和代码审查组织在可调整的三栏界面中，使用系统 WebView，不引入 Electron。

> **开发阶段：Phase 2 起步。**真实模型聊天和只读文件浏览已经实现；自主代码修改、命令执行和原生 MCP 集成尚未开放。部分界面用于明确标注的 Mock 演示。

## 界面预览

![Maestro-Agent workbench](artifacts/ui/maestro-workbench.png)

<details>
<summary>查看 Explorer / 代码预览</summary>

![Readonly Explorer preview](artifacts/ui/maestro-explorer.png)

</details>

以上为项目自带 Mock 数据的浏览器截图，展示布局与交互，不代表真实工作区、模型执行或原生桌面验收结果。

## 功能状态

| 功能 | 当前状态 |
| --- | --- |
| OpenAI 兼容聊天 | Rust 请求 `/chat/completions`，SSE 增量回复、停止生成、超时与错误提示 |
| 本地会话 | SQLite 持久化多轮对话、运行结果和内部审计，恢复异常中断记录 |
| 桌面生命周期 | Tauri 单实例、单个活动运行、取消与退出排空 |
| 工作区 Explorer | 显式选择目录，读取真实文件树和 UTF-8 文本，Monaco 只读预览 |
| Review / 审批 | 提议 Diff 与单次计划决定；批准不应用修改，真实模型暂不自动生成审批卡片 |
| IDE 工作台 | 可拖拽分栏、会话搜索与项目分组、Settings、应用内快捷键 |
| Activity / 参数控件 | Dashboard、Tokens meter、上下文、Temperature 和主题选择包含明确 Mock |
| Provider / 扩展 | Anthropic/Ollama 选择与 MCP/Skills/Plugins 状态为 UI 演示，不启用原生协议或工具 |

**No model tool execution, project-file writes, patch application, or command execution is enabled.** Selecting Execute or approving a plan does not bypass this boundary.

## 快速开始

### 1. 获取项目

```powershell
git clone https://github.com/fenggezaici/Maestro-Agent.git
Set-Location Maestro-Agent
```

前端使用 **Node.js 24 + pnpm 12**，版本由 `packageManager` 和 `pnpm-lock.yaml` 固定。

### 2. 只看前端 UI

无需 Rust、WebView2 或 API Key：

```powershell
corepack.cmd enable pnpm
pnpm.cmd install --frozen-lockfile
pnpm.cmd dev:ui
```

自动打开 **http://127.0.0.1:1420/?preview=1**。这是隔离的本地 Mock 预览：可以切换项目、调整布局并创建模拟会话，但不能发送真实聊天或读取磁盘。

如无法启用 pnpm shim，可直接使用 `corepack.cmd pnpm install --frozen-lockfile` 和 `corepack.cmd pnpm dev:ui`。非 PowerShell 环境可使用对应的 `pnpm` / `corepack` 命令。

### 3. 运行 Windows 桌面客户端

另需 Rust MSVC 工具链、Visual Studio C++ Build Tools / Windows SDK，以及 WebView2 Runtime：

```powershell
pnpm.cmd tauri dev
```

进入左下角 **Settings → Models**，填写并保存：

- **Base URL**：通常为 `https://your-provider.example/v1`，Rust 自动补全 `/chat/completions`。
- **API Key**：服务提供的访问密钥。
- **Model**：该服务支持的模型名称。

输入消息并发送，或按 **Ctrl/Cmd+Enter**。右侧 **Explorer** 在开发模式下默认浏览当前仓库，也可通过路径栏选择其他本地项目。

更新 Rust 后，先关闭旧窗口、在启动终端按 Ctrl+C，再重新运行 `pnpm.cmd tauri dev`。当前开发与打包流程面向 Windows；其他平台尚未完成验证。

## 安全与数据边界

> **API Key 明文保存在 WebView localStorage，不是系统凭据库。**仅在信任的本机使用，优先配置 HTTPS；开发窗口和安装版的 origin 不同，需要分别保存配置。

- 配置凭据不会主动写入会话数据库、审计或日志；模型错误文本会脱敏。
- 会话数据库包含用户消息与模型回复，请视为私人数据。位置为 Tauri `app_data_dir()/maestro.sqlite3`。
- 所有 SQLite 操作由 `maestro-sqlite` 专用 OS 线程独占，通过有界 `mpsc` 与异步调用者通信；不会在异步线程直接执行阻塞 SQL。
- Rust 持有模式与运行授权；Plan Mode 拒绝副作用工具和未知名称，但允许应用内部会话/审计落库。
- Workspace reads run separately in `spawn_blocking`, skip `.git`, `node_modules` and `target`, and enforce path, link, scan-budget and file-size limits.
- 文件浏览不会自动把内容发送给模型。只读检查不是完整 OS 沙箱；并发恶意文件系统下的残余 TOCTOU 风险见[工作区文档](docs/workspace-filesystem.md)。

真实聊天上下文由 Rust 从指定会话的已完成记录重建，附加模式提示。前端使用带运行/会话 ID 与序号的 IPC 事件，避免串流和重复追加。

## 技术架构

```text
React 19 + TypeScript + Vite + Tailwind CSS 4
Radix UI / Lucide / Monaco / react-resizable-panels
					  │ Tauri IPC: invoke + events
					  ▼
Rust kernel ── model transport (reqwest / SSE)
	 │       └─ readonly workspace service (spawn_blocking)
	 └─ bounded mpsc ── SQLite OS thread
```

| 路径 | 职责 |
| --- | --- |
| `src/components/` | 工作台、聊天、设置、Review / Explorer 与 UI 演示 |
| `src/preview/` | 与真实 IPC/凭据隔离的前端预览数据 |
| `src/ipc.ts`, `src/useKernel.ts` | 聊天契约、事件缓冲与会话状态 |
| `src/workspaceApi.ts`, `src/useWorkspace.ts` | 工作区读取与请求竞态保护 |
| `src-tauri/src/kernel/`, `agent/` | 运行生命周期、模型编排与取消 |
| `src-tauri/src/llm.rs` | OpenAI 兼容 HTTP/SSE 与 Rust System Prompt |
| `src-tauri/src/db.rs` | SQLite 线程、迁移、恢复与事务 |
| `src-tauri/src/workspace.rs` | 受限文件树与文本读取 |
| `src-tauri/capabilities/`, `permissions/` | 本地主窗口的最小命令权限 |

## 测试与构建

```powershell
pnpm.cmd build
pnpm.cmd test
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Rust 测试包含本地 OpenAI 兼容 SSE 服务、取消/错误、会话恢复，以及真实目录与 Windows junction 场景；不需要云端 API Key。前端组件测试与浏览器截图检查不等于完整原生 UI 验收。

生成可重复的 Mock 截图：

```powershell
pnpm.cmd exec playwright install chromium
pnpm.cmd screenshot
```

Chromium 仅是开发测试工具，不进入桌面运行时，也不引入 Electron。原生生命周期检查可使用 `scripts/smoke-desktop.ps1 -NativeOnly`；完整模型 UI 测试需要有效配置及显式 `-AllowModelRequest`，可能产生费用。

Windows 打包：

```powershell
pnpm.cmd tauri build
```

NSIS 输出位于 `target/release/bundle/nsis/`。优先复用系统 WebView2，缺失时下载 bootstrapper，不捆绑完整 Chromium。打包工具缓存位于 `target/.tauri/`。安装包大小和内存占用请以当前版本实测为准；低内存目标尚未完成验收。

## 文档与后续方向

- [真实模型配置、重启与流式协议](docs/llm-streaming.md)
- [工作区文件读取和安全限制](docs/workspace-filesystem.md)
- [IDE UI 控件与预览边界](docs/ide-ui-upgrade.md)
- [UI 设计、Activity 与截图](docs/ui-redesign.md)
- [初始阶段验证记录（历史）](docs/verification.md)

后续重点：明确授权的模型文件上下文、真实计划与差异生成、执行前文件基线核验、细粒度工具审批、原生 MCP/扩展，以及系统凭据保护。附件和自动任务尚未接入后端。

## License

当前仓库尚未指定项目许可证；公开源码不等于授权自由使用、修改或再分发。依赖组件遵循各自许可证。
