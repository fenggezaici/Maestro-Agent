# 沉浸式 IDE 工作台 UI

本次仅重构前端视觉与交互，没有新增 Rust 调试、模型或工具执行逻辑。

## 设计与组件

- Canvas `#171717`, sidebar Surface `#1e1e1e`, code surface `#1b1b1b`. Tonal backgrounds and spacing replace hard panel borders.
- 初始 240px 侧栏、弹性对话区和约 45% Monaco 审查区；三栏可实时拖拽，尺寸在本地保存。
- 使用 react-resizable-panels v4 的 Group / Panel / Separator（Group 在组件内别名为 PanelGroup），手柄仅 1px 可视线，提供扩大的拖拽命中区域与键盘调整。
- 窄窗口使用嵌套纵向分栏，移动端单列；Monaco 自动调整尺寸，隐藏/恢复和断点切换保留文件及 Diff 模式。
- 会话按 Workspace 使用 Radix Accordion 分组，保留展开状态；搜索临时展开匹配组，清空后恢复。
- Activity 采用单行导航入口，点击后只切换中间工作区，不打开 Modal 或覆盖右侧 Diff。
- Dashboard 包含四项 KPI、六个月 GitHub 风格热力图与 Recharts 30 日 Token 折线图；所有统计明确为 Mock。
- 本地 Inter 字体、Lucide 细线图标、Radix Select / Tabs / Tooltip / DropdownMenu / Dialog。
- 圆角输入区、低对比消息、分步计划卡片、实心 Approve 与幽灵 Reject。
- Monaco 真正只读，支持文件标签、并排/行内视图和柔和红绿高亮；切换与卸载时释放编辑器及模型。
- 编辑器与少量语法语言按需加载；本地 worker 和字体不依赖 CDN，也不需要 `blob:` worker 或 eval。

## 预览和真实状态

`npm run tauri dev` 始终使用上一阶段的 Rust IPC、会话状态、取消和计划审批。

独立前端预览：运行 `pnpm install` 后执行 `pnpm dev:ui`，自动打开 `http://127.0.0.1:1420/?preview=1`。
首次启用 pnpm 可运行 `corepack enable`；不启用 shim 也可使用 `corepack pnpm install` 和 `corepack pnpm dev:ui`。
该路由仅在浏览器生效，标记为 Frontend preview / 示例数据。
可以在当前选中项目新建本地模拟会话；不会调用 invoke/listen，也不会写入真实会话库。
发送、模式变更及批准/拒绝仍禁用，项目分组及热力图不是后端能力。
示例对话与 C++ Diff 是原创 Mock 内容，不代表工作区真实读取或模型执行。
普通浏览器访问没有预览参数时仍显示真实后端连接错误，不进行静默模拟。

预览的数据源为 Projects → SessionDetail 的嵌套结构，扁平会话只作为兼容视图派生。模拟项目可切换，但真实客户端当前仅有一个本地 Workspace。
Automations and attachments are not connected to the backend. Unsupported actions stay explicitly labeled or disabled.
Tauri Settings now saves API Key, Base URL and Model in localStorage for real Rust streaming. Browser preview neither loads nor saves live credentials; see [LLM streaming](llm-streaming.md).

## Activity 主工作区路由

点击侧栏 `Activity` 后，中间面板切换至 `Activity Dashboard`，入口显示选中状态。
点击 `Back to Chat`、`Agents`、具体 Session 或新建会话后恢复聊天。
视图切换不销毁 Rust IPC 控制器，正在生成的回复继续接收；聊天草稿、滚动位置、Diff 文件/模式和分栏尺寸保留。
看板不会切换权限模式、调用统计/账单接口或把真实会话数量包装为修改数据。

统计样本固定为 UTC `2026-05-09` 至 `2026-11-08`，不依赖当前时间或随机数。
KPI、每日行数、Token 和计划数总量一致；30 日图表取同一日序列的最后 30 个样本。
即使外壳已连接 Rust，看板也标注 Mock，不能当作实际代码活动、用量或账单。
Recharts 按需加载，热力图在窄面板内横向滚动，不挤出工作区。

## 测试与截图

先运行 `pnpm build`，再运行 `pnpm test` 和 `pnpm screenshot`。
首次截图需要 `pnpm exec playwright install chromium`；Chromium 仅用于开发验证，不打入 Tauri 安装包，不引入 Electron。

截图保存在：

- `artifacts/ui/maestro-workbench.png`：1600 × 1000 三栏工作台。
- `artifacts/ui/maestro-workbench-1280.png`：1280 × 900 桌面布局。
- `artifacts/ui/maestro-workbench-stacked.png`：900 × 900 下方 Diff 布局。
- `artifacts/ui/maestro-workbench-resized.png`：实际拖拽调整后的布局。
- `artifacts/ui/maestro-activity.png`：看板热力图的逐日悬浮摘要。
- `artifacts/ui/maestro-workbench-mobile.png`：640px 单列布局。
- `artifacts/ui/maestro-dashboard.png`：1600px 主工作区 Dashboard。
- `artifacts/ui/maestro-dashboard-1280.png`：1280px 看板。
- `artifacts/ui/maestro-dashboard-stacked.png`：下方 Diff 布局中的看板。
- `artifacts/ui/maestro-dashboard-mobile.png`：640px 单列看板。

截图脚本启动临时生产预览服务，应用桌面生产 CSP，验证 Dashboard 路由、无 Modal、真实 Recharts、六个月热力图、右侧面板保持及既有拖拽/响应式行为，随后关闭浏览器和服务器。
截图是静态示例的真实浏览器渲染，不替代原生桌面完整功能验收。

## 安全与体积

Monaco 0.57 的 DOMPurify 依赖及内嵌副本均重定向至已修复的 3.4.17；仅 npm override 不足以替换内嵌代码。
后续升级 Monaco 时须复核 `vite.config.ts` 中的别名及兼容性，并重新执行 npm audit、构建与真实编辑器测试。
Monaco adds lazy-loaded assets. It is not initialized in sessions without a Diff; the previous Phase 1 installer size is no longer claimed for this UI build.

