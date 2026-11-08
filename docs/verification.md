# Phase 1 初始工作台验证记录（历史）

本页记录真实模型接入前的工作台阶段；测试数量、产物尺寸和占位模型限制不代表当前版本。
For the current real-model implementation, configuration and tests, see [LLM streaming](llm-streaming.md).

环境：Windows x64，Node.js 24，Rust MSVC。日期：2026-11-08。

## 已通过

- React/TypeScript 生产构建。
- 前端 16 项测试：工作台布局、会话创建/选择、流式消息去重、初始化缓冲、审批/拒绝、会话查询竞态及 StrictMode 清理。
- Rust 24 项测试：精确工具白名单、伪装/未知工具拒绝、真实分发拦截、会话迁移、计划持久化/单次审批、取消失效、存储故障禁用及原有生命周期/SQLite/IPC 覆盖。
- Clippy 全 workspace / 全 targets 检查，警告视为错误。
- Windows release 编译和 NSIS 安装包生成。
- 原生窗口启动、重复启动仅保留单实例、正常退出。

安装包：`target/release/bundle/nsis/Maestro-Agent_0.1.0_x64-setup.exe`，约 **1.72 MiB**（1,805,073 bytes）。
桌面程序：`target/release/maestro.exe`，约 **4.29 MiB**。
打包不捆绑完整 WebView2 Runtime，缺少运行时的机器需要下载引导安装。

## 未通过或未验证

- Phase 1 完整原生 UI 端到端验收尚未取得通过结果；此前环境的 UI Automation 子节点数量为 0。本次验证包括组件/IPC 模拟测试和原生生命周期，不将它们当作完整 UI 验收。
- `scripts/smoke-desktop.ps1 -NativeOnly` 仅验证原生生命周期，不替代真实界面和 IPC 验证。
- 未执行安装包实际安装/卸载验收。
- 当前使用明确标注的 Rust 占位模型，尚未接入真实服务、读取工作区或执行文件/命令工具。
- 已实现持久化多轮会话与具体计划审批；批准仅记录决定，不切换 Execute Mode、不应用 Diff，也不复用旧运行授权。
- Diff 是占位模型生成的提议内容，不是工作区真实差异。

## 内存目标

本机 Phase 1 原生测试中，应用及 WebView2 子进程工作集总和采样为 **291.9 MiB**。
该指标可能重复计入共享页，也不是私有常驻内存；它不能证明已达到“低内存”目标。
后续应建立明确内存预算，并测量私有工作集、空闲/流式输出时的峰值与稳定值，再进行优化。

本阶段完成工作台、Rust 工具白名单拦截、占位流和计划审批，不宣称真实模型/执行工具或低内存验收已经完成。
