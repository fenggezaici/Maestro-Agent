# Core interactions and model capabilities

## Run and verify

```powershell
Set-Location E:\Maestro-Agent
pnpm.cmd install
pnpm.cmd dev:ui
```

Browser preview supports UI/theme/model-capability exploration without Rust. For real streaming close the old app/server, run `pnpm.cmd tauri dev`, then configure **Settings → Models → Provider**. Existing v1 connection settings are preserved and migrated into the OpenAI-compatible profile when provider profiles are created.

## Dynamic models

`modelCapabilities.ts` defines catalog capabilities, not a remote model-discovery API. It includes provider defaults, context limits, optional vision metadata and reasoning controls.

- o3-mini/o3: Low/Medium/High effort, 200k catalog context.
- gpt-4.1: approximately 1M catalog context, no effort selector.
- gpt-5.2: mapped Low/Medium/High/Extra High effort.
- Claude 3.7: Low/Medium/High mapped to Anthropic thinking budgets.
- DeepSeek R1 / reasoner: a visible **Thinking · 内置推理** indicator and 128k reference context, without an unsupported effort selector. `deepseek-r1`, `DeepSeek-R1` and `deepseek-ai/DeepSeek-R1` are recognized for metadata; the configured request model ID is never rewritten into `deepseek-reasoner`.
- Tagged/distilled R1 variants retain reasoning identity, but do not inherit an unverified 128k context. Ollama context always depends on the local deployment (`num_ctx`), not the hosted catalog limit. The UI shows **Context · 服务端配置** and an explanation, preserving R1's built-in reasoning indicator. Unknown custom models are not assigned an invented context window.

Both Composer and **Settings → Models → 模型能力 · Context & Thinking** use the same capability controls. Select an adjustable model such as OpenAI `o3-mini` or Anthropic Claude 3.7 to configure supported effort/budget levels. Settings saves the chosen level; Composer applies it immediately after successful storage. A failed save does not leave a falsely selected strength in the UI. Fixed reasoning does not mean an adjustable effort parameter is available, nor that reasoning text is streamed separately.

Context tooltips give the numeric reference limit and distinguish it from provider usage and deployment configuration. These labels do not change Rust history limits or configure an Ollama server. On narrow Composer panels, container queries progressively hide labels and cap selector widths instead of wrapping the toolbar.

Unknown strengths are never manufactured merely to fill a dropdown. The registry labels Max as a possible UI value but exposes it only if a supported model mapping is added; no current transport pretends to support an unconfigured Max effort.

OpenAI and DeepSeek use Chat Completions SSE. Anthropic uses Messages, `x-api-key`, `anthropic-version`, system-role mapping and its SSE event structure. Ollama uses its `/v1` compatibility interface with an optional key. Credentials are isolated per provider and remain plaintext localStorage, not a secure OS vault.

## Modes

Rust owns selectable Plan / Code / Confirm / Auto; Plan is the backend default.

- Plan: read-only policy; side-effect tools are stripped from the model's tool list.
- Code: reads run immediately; write/patch/command tools suspend for approval, then execute for real.
- Confirm: every tool/command suspends for explicit per-call confirmation, then executes.
- Auto: already-authorized tools execute directly, including writes and commands.

Approving a plan only records intent — it never applies a diff by itself. Tool-level write/command calls are what produce approval cards; rejected or cancelled approvals leave the workspace untouched.
Schema v3 transactionally rebuilds the mode constraint while preserving IDs/high-water state, legacy Plan/Execute rows, sessions, audits and plan decisions. Old Execute remains history-only, not a new selectable option.

## Usage and media

Turns counts actual displayed requests. Token fields use provider-reported usage where available; otherwise an explicitly marked heuristic estimate is shown, including in preview. Estimates are not tokenizer counts and do not enforce the catalog token limit. Rust history still has its existing bounded context construction.
Rust measures HTTP/stream elapsed time and first content latency. Generation speed uses reported output tokens and elapsed generation time; fallback speed is labeled estimated, or remains unknown. SSE fragments are never counted as tokens. Metrics are stored in SQLite through its dedicated OS thread.

Images and videos are selected locally, limited to six files and 20 MiB each. Object URLs are revoked on removal/app unmount. Sending retains thumbnails as **not uploaded** message placeholders for the current app instance; no bytes, URLs or media metadata enter model IPC or persisted messages. An image-only request still requires a textual message. Media is not multimodal model inference.

Assistant Markdown disables raw HTML and automatic remote images. Code fences show language, syntax highlighting and a clipboard action; user bubbles expose Copy on hover/focus.

Dark/Light/System themes apply across the document, body-portaled menus and Monaco; system listeners are cleaned up. Editor font/size also populate CSS code-font variables for Markdown and readonly previews. Language changes the basic frontend/navigation/settings labels, not model-generated text or backend error language.

## Validation

`pnpm.cmd build`, `pnpm.cmd test`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `pnpm.cmd screenshot`.
Local HTTP tests cover OpenAI usage and Anthropic text/thinking-budget/usage conversion without cloud credentials. Browser captures verify actual white Light theme, dynamic effort selection, popup hit-testing and unchanged workspace layouts. Actual cloud model availability/account limits still require your own valid configuration.

R1/context correction: **70 frontend tests**, production TypeScript/Vite build and Chromium screenshot suite passed. The added coverage exercises aliases, provider-specific strengths, R1 local deployment metadata, Settings persistence and storage-failure rollback. Reproduction screenshots: `artifacts/ui/maestro-r1-context.png`, `maestro-thinking-strength.png`, `maestro-model-capabilities.png`. They use isolated frontend preview data, not cloud inference.



## Timeline, tool rows and stop

- Reasoning bursts, tool calls and assistant text share one ordered timeline. Consecutive reasoning/tool items form an execution phase (`已工作 X 秒`); assistant text is rendered at the exact point it arrived instead of being appended after the whole turn.
- An execution phase starts expanded while the run is active and collapses when the phase ends or the run reaches a terminal state. Users can toggle it manually; manual state is not overridden.
- Tool and reasoning rows expose a right-side chevron with `aria-expanded`. Finished tool rows are intentionally quiet (gray text/icons) and only brighten on hover; `已执行` is not rendered, while failed/awaiting calls keep a status label.
- Clicking a file target opens the right-side Explorer and loads the file in Monaco. Historical runs use the same `onOpenFile` path as live runs.
- Stop sends `abort_chat` and immediately seals the visible reasoning/tool state; there is no custom "stopping" pill. The final cancelled state is driven by the backend terminal event. The Rust kernel keeps a `cancelled_run` flag and a 250ms stream poll so cancellation does not depend on a single watch notification.

Composer stats follow the DeepSeek Harness pattern: two centered pills below the composer, provider-reported usage when available, and an explicit `est.` marker for live output estimates. Cache hit is cache-read tokens over billed prompt input; output tokens are excluded, and a missing provider cache bucket shows `—` rather than an invented value.

A run is capped at 64 model→tool→model rounds by default (`MAESTRO_MAX_TOOL_ROUNDS`, minimum 8). When the cap is reached, the next turn is tool-free and explicitly asks the user to send “继续” to start a new run; this prevents silent partial completion.

Code mode treats workspace writes as normal work; only a conservative dangerous-command list requires approval. Read-only tool calls in one batch run concurrently, while writes and commands stay serial. `spawn_subagent` runs a read-only research child loop and returns its summary to the parent model.

Model reasoning levels come from the models.dev catalog (`reasoning_options[type=effort].values`); the model config dialog defaults to smart config and only shows the composer slider when more than one level is resolved. Advanced config can override input types, capabilities, ordered levels, and the reasoning mapping JSON.

Provider requests retry before the first token using the opencode policy: 5 attempts, 2s initial delay, x2 backoff, 25% jitter, 30s cap, honoring Retry-After; 401/403/404/400 are not retried, and a stream that already emitted tokens is not replayed.
