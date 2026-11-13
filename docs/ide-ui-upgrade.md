# IDE UI upgrade

## Preview on Windows

```powershell
Set-Location E:\Maestro-Agent
pnpm.cmd install
pnpm.cmd dev:ui
```

Open `http://127.0.0.1:1420/?preview=1`. This preview needs Node.js 24 but not Rust or a model API key.
If pnpm is not enabled, run `corepack.cmd enable pnpm`, or use `corepack.cmd pnpm install` and `corepack.cmd pnpm dev:ui`.
For real model testing use `pnpm.cmd tauri dev`, then **Settings → Models**. Browser preview does not read or write live credentials.

## Interaction map

- Composer: compact **计划 / 代码 / 确认 / 自动** 模式、模型与上下文控制同排；输入区最大 1080px，宽于 896px 对话正文。Preview model selection is local only; live model selection preserves Key/Base URL.
- Context/Temperature: explicit Mock controls. The `2.4k / 8k` meter is illustrative, not measured token usage. These controls do not alter real IPC parameters or the 8192-byte input limit.
- Settings: fixed sidebar-bottom entry, independent of the scrolling project list, including mobile layout. Its category rail remains **General / Appearance / Models / Shortcuts**.
- Command Center: `Ctrl/Cmd+K` opens a top quick-pick overlay (the `search` shortcut binding). Commands / Sessions / Files sections share one fuzzy match and arrow-key navigation; Enter runs, Esc closes. Files reuse the already-loaded workspace tree (Explorer data, read-only); selection opens Explorer and previews the file. Recent queries persist per source in localStorage; pinned sessions only reorder the local list and are not synced to the backend.
- Sessions: single-line rows (status + title + relative time); full title, run count and `#id` live in the tooltip and aria-label. Lists group by local day (置顶 / 今天 / 昨天 / 7 天内 / 更早); pinning is a local UI preference.
- Plan cards: pending plans render expanded; approved/rejected/invalidated plans collapse to a summary row and can be reopened manually. Decisions remain intent-only; nothing is applied.
- Composer thinking strength: the model picker exposes a continuous effort slider for adjustable models; fixed-reasoning models show a static line. Settings → Models keeps the detailed select.
- Appearance: font family/size update readonly Monaco previews. Dark/Light/System are real themes; 界面缩放 keeps the CSS zoom behavior (default 100%).
- Providers: OpenAI/Anthropic/Ollama selector demonstrates provider UI only. Actual Rust transmission remains OpenAI-compatible; native Anthropic/Ollama protocols are not enabled by this selector.
- Shortcuts: search, click a binding, then press a modified key combination. Escape cancels capture. Conflicts and unsupported chords are rejected; Ctrl/Cmd+Enter remains reserved for Send. These bindings are application UI actions, not OS global shortcuts.
- Right pane: closable **文件 / 变更 / 任务** tab strip with a trailing **+** menu; default opens 文件 + 变更, Tasks appears when a plan exists. Changes use the ZCode lightweight unified diff (14% row tint + 3px inset bar, 18% gutter tint), not Monaco Diff. In Tauri, Explorer reads the selected local directory and file text through Rust. Browser preview retains explicitly labeled samples.
- Extensions: top tabs are **技能 / MCP 服务 / 发现 / 配置** — Skills and MCP each get a dedicated page. Discover installs report errors inline on the card; installed items show a disabled "已安装" state (MCP by server name, skills by name + a localStorage record of imported repos).
- Appearance: **界面缩放** (100% / 125% / 150%) scales the whole workbench via CSS zoom; default is 100%. The Extensions page itself uses larger base font sizes (descriptions/state/links bumped from 8–11px to 10–13px) rather than relying on global zoom.
- Model inputs are free text (no preset lists) with `<datalist>` suggestions from **fetch_models**: `GET {base}/models` for OpenAI-compatible providers (ids only — context stays unknown honestly) and `/api/tags` + `/api/show` for Ollama (real `*context_length`). Results cache per provider for 24h and refresh silently in the background when credentials exist; saving the Models form also triggers a refresh.
- Capability resolution order: **user override → provider-fetched → models.dev catalog → built-in table → unknown**. The `model_catalog` command pulls the public models.dev api.json (cached 24h client-side) — the same metadata source opencode-style tools use. Settings → Models offers a per-model 能力覆盖 (context tokens + reasoning kind) for custom gateway deployments. Provider HTTP errors now surface the response body (redacted, 300 chars) so "Model Not Exist" is visible instead of a generic hint.

UI preferences use source-separated `maestro.ui.v1.live` / `maestro.ui.v1.preview` keys, separate from real `maestro.llm-settings.v1` credentials. Panel, Diff and Explorer selections survive view changes and panel hiding; mock extension states last for the current app instance.

## Validation and screenshots

Run `pnpm.cmd build`, `pnpm.cmd test`, and `pnpm.cmd screenshot`.
First-time browser installation: `pnpm.cmd exec playwright install chromium`. The browser is a development test tool, not an Electron dependency or bundled app runtime.

New captures:

- `artifacts/ui/maestro-workbench.png`
- `artifacts/ui/maestro-composer-controls.png`
- `artifacts/ui/maestro-settings.png`
- `artifacts/ui/maestro-explorer.png`
- `artifacts/ui/maestro-extensions.png`

Browser checks retain production CSP, local fonts/workers, resizable layouts, Activity routing and static preview safeguards. They also verify the fixed footer while sessions scroll, flat composer controls, Monaco previews, Settings menus, shortcuts and extension UI transitions. Filesystem reads are tested in Rust against real files, including this repository.

- Task tracker: `update_plan` steps stream into the right-panel Tasks tab; the conversation header shows a compact `任务 N/M` chip instead of an overlay card. Completed steps are gray + struck through.
- Composer stats: two DeepSeek Harness-style pills below the composer (activity and usage); cache hit is provider-reported cache reads over billed input, never guessed.
- Branding: sidebar uses the two-tone `MaestroLogo`; desktop icons are generated from `app-icon.svg` via `tauri icon`.

- Model config: **智能配置** follows models.dev context/reasoning levels; advanced mode edits input types, capabilities, ordered reasoning levels and the reasoning parameter mapping JSON. The composer slider is shown only when the resolved level list has more than one entry.
- Model transport: connect/TLS/5xx/429 failures retry with the opencode policy (5 attempts, 2s initial, x2 backoff, 25% jitter, 30s cap, Retry-After honored) before the first token.
