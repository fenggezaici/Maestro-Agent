# Maestro Agent 工作规范

## UI/布局

- **参考Zcode对应UI和布局**：所有 UI 设计、组件布局与交互模式以 ZCode（zai-org/ZCode）对应模块为准。
- ZCode 参考克隆位于仓库外 `D:\zcode-ref`（浅克隆）；改 UI 前先对照其 `packages/ui` 下同名组件（CommandCenter、chat-input-toolbar、ToolCallBlocks、WorkspaceSidebar 等）。
- 不引入 ZCode 依赖本体；参考其布局与交互，用本项目现有 Radix/Tailwind 实现。

## 边界

- Preview（`?preview=1`）不调用 `invoke/listen`；live 专属能力（原生对话框、IPC）在 preview 下禁用，不造假。
- Explorer 只读；工具执行、文件写入、命令执行未开放，不伪造后端能力。
- 未知模型的 context/thinking 不编造数字，显示"服务端配置"。
