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

- Composer: compact **Plan / Execute**, model, context choice and Tokens displayed inline in one toolbar; no intermediate parameter Popover. Preview model selection is local only; live model selection preserves Key/Base URL.
- Context/Temperature: explicit Mock controls. The `2.4k / 8k` meter is illustrative, not measured token usage. These controls do not alter real IPC parameters or the 8192-byte input limit.
- Settings: fixed sidebar-bottom entry, independent of the scrolling project list, including mobile layout. Its category rail remains **General / Appearance / Models / Shortcuts**.
- Appearance: font family/size update readonly Monaco previews. Dark/Light/System options are UI Mock choices; the workbench deliberately retains its dark visual style.
- Providers: OpenAI/Anthropic/Ollama selector demonstrates provider UI only. Actual Rust transmission remains OpenAI-compatible; native Anthropic/Ollama protocols are not enabled by this selector.
- Shortcuts: search, click a binding, then press a modified key combination. Escape cancels capture. Conflicts and unsupported chords are rejected; Ctrl/Cmd+Enter remains reserved for Send. These bindings are application UI actions, not OS global shortcuts.
- Right pane: **Review / Explorer** tabs. In Tauri, Explorer reads the selected local directory and file text through Rust. Browser preview retains explicitly labeled samples. Review remains proposed changes and never overwrites actual disk previews. See [Phase 2 workspace reads](workspace-filesystem.md).
- Extensions: **MCP / Skills / Plugins** status tabs and local simulated installation/enabling. No package downloads, processes, MCP connections or model tools are started.

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
