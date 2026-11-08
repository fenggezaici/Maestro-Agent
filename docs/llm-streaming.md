# 真实模型流式聊天

## 重启并测试

1. 关闭旧 Tauri 窗口，并在原启动终端按 Ctrl+C，释放 Vite 的 1420 端口。
2. 在项目目录执行：

```powershell
Set-Location E:\Maestro-Agent
pnpm install
pnpm tauri dev
```

未启用 pnpm shim 时，用 `corepack pnpm install`、`corepack pnpm tauri dev`。
需要 Rust MSVC、C++ Build Tools / Windows SDK 和 WebView2。Cargo 会自动拉取新增 Rust 依赖。

3. 在桌面客户端 Settings 中填写并保存 API Key、Base URL、Model。
4. Base URL 通常为 `https://your-provider.example/v1`；Rust 自动追加 `/chat/completions`。本地 HTTP 模型也支持。
5. 输入消息并发送或 Ctrl/Cmd+Enter。用户消息先出现，Agent 从“思考中...”开始增量显示；停止按钮会取消网络读取。

**不要用 `pnpm dev:ui` 验证真实模型**：它是浏览器 Mock 预览，不调用 Rust，也不读取/保存真实凭据。
开发客户端和安装版的 origin 不同，localStorage 配置需要分别保存。

## 配置与安全

- API Key 按要求存入 `maestro.llm-settings.v1` 的 localStorage，属于明文，不是安全凭据库。仅在信任的本机使用。
- Key 默认遮罩，可清除；存储不可用时明确报错，不伪装保存成功。
- 凭据仅通过 invoke 交给 Rust，不进入 SQLite 会话/审计、事件载荷或日志；模型错误文本会脱敏。
- Rust reqwest sends HTTP requests; no browser CORS permissions, HTTP plugin, or relaxed frontend CSP is needed.
- 禁止 URL 内嵌账号、查询参数、片段；不跟随重定向，避免认证信息被带到其他服务。
- 连接超时 20 秒、单次 token 等待 60 秒、请求上限 600 秒；流/frame 和回复有大小上限。

## 命令与事件

命令注册在本项目实际的 `src-tauri/src/lib.rs`；`main.rs` 仅调用库启动。
`stream_chat` 的前端参数为 `messages`、`api_key`、`base_url`、`model`、`session_id`。
`session_id` 绑定当前工作台会话；直接调用时可省略，由 Rust 创建新会话。命令返回接受后的 run ID。
`messages` 只接受 user/assistant。Rust 从该会话的已完成记录重建可信上下文，最多 32 轮/128 KiB，并将当前消息只插入一次。
Plan Mode 系统提示由 Rust 根据该轮不可变模式添加：“你处于只读计划模式”。当前不执行模型工具调用。

为避免多会话串流，聊天事件采用带关联信息的载荷，而不是无法关联的裸字符串：

- `chat-token`：`{ sequence, sessionId, runId, token }`。
- `chat-end`：`{ sequence, run }`，完成/取消后、事务落库后发送。
- `chat-error`：运行失败时 `{ sequence, run, message }`；命令参数错误时为脱敏错误字符串。

Chat events and `kernel-event` share one sequence-deduplicated frontend reducer; text is not appended twice from separate channels.
先注册所有监听再查询快照；支持 token 早于 invoke 返回、重载缓冲、切换会话、Dashboard 后台接收和 StrictMode 清理。
失败会结束对应运行并弹 Toast；HTTP 错误不让内核整体离线。存储失败保留运行门禁，明确禁用新工作并等待重启恢复。

真实模型仅生成聊天文本，不自动制造审批卡片或声称已修改文件。旧计划/示例 Diff 和 Activity Dashboard 保留现有语义。

## 自动验证

`pnpm build`、`pnpm test`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`。
Rust 集成测试使用本地 OpenAI 兼容服务，覆盖真实 HTTP、先后分段输出、中文 UTF-8、只读 Prompt、持久化、401 和取消，不需要外部 Key。
完整原生 UI 验收仍需自己的有效服务配置；不宣称已测试未提供的云端账号。
`smoke-desktop.ps1 -NativeOnly` 不调用模型；完整模式需显式 `-AllowModelRequest`，可能产生 API 费用。

