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

Subagent definitions are resolved from three sources with `builtin < global < project` precedence: built-ins (`explore`, `general`), `~/.maestro/agents/*.md`, and `<workspace>/.maestro/agents/*.md`. The settings panel lists the merged view with a source badge, locks the name of an existing definition, and lets a built-in be overridden by a global file (deleting that file restores the built-in). The `spawn_subagent` enum is generated from the same merged list, so a definition saved in Settings is selectable by the model without editing code; `hidden: true` and `mode` other than `subagent` stay out of the enum.

The child loop never inherits the parent's authorization: its tool calls are filtered against the tools offered to that agent and executed under the Plan gate, so `write_file`/`apply_patch`/`run_command` are denied even when the parent run is in Code or Auto. A consequence is that read-only child calls do not suspend for per-call approval in Confirm mode; the parent's approval slot is not consumed by research work.

Model reasoning levels come from the models.dev catalog (`reasoning_options[type=effort].values`); the model config dialog defaults to smart config and only shows the composer slider when more than one level is resolved. Advanced config can override input types, capabilities, ordered levels, and the reasoning mapping JSON.

Provider requests retry before the first token using the opencode policy: 5 attempts, 2s initial delay, x2 backoff, 25% jitter, 30s cap, honoring Retry-After; 401/403/404/400 are not retried, and a stream that already emitted tokens is not replayed.

Composer tok/s uses provider-reported output tokens and provider timing only: `sum(outputTokens) / sum(elapsedMs - firstTokenMs)` across the session's runs. It never derives speed from estimated text; when usage is missing the pill shows `—`.
The bottom agent status pill shows `等待批准 / 执行工具中 / 思考中 / 工作中` plus live elapsed time (seconds below a minute, minutes + seconds above), driven by run metrics or `createdAt`.

While a run is stopping, new text is queued instead of starting a second run; the queue starts after the old run's terminal status. Late content/status events from the old run are ignored and cannot clear the new active run. Stopping also settles the UI immediately: the working pill and stop button disappear before backend acknowledgement.
Change cards are per turn: `run_change_summary` is the header, `run_changes` loads details on expand, `preview_revert_run_changes` classifies safe/unsafe/ignored files, and `revert_run_changes` applies only safe files. A reverted turn keeps its card with a “reverted” badge.

Projects are ordinary rows: the project seeded on first run (`Maestro-Agent`) can be renamed, archived, restored or deleted like any other, and deleting one cascades its sessions, runs, plans, metrics and audit rows without a way to undo. A new session without an explicit project lands in the oldest unarchived project; when every project is archived or deleted the backend refuses with `请先新建项目` and the “新建会话” entry is disabled. Deleting a project asks once through the Workbench dialog; only session deletion keeps its own inline confirmation.

Extensions have a persisted lifecycle. MCP servers live in `~/.maestro/mcp.json`; the UI switch writes `disabled` and start/stop the child, and app startup restores every entry with `disabled: false` in the background (a slow `npx` download never blocks the window). On Windows the launcher is resolved through `PATH`/PATHEXT so `.cmd` shims such as `npx` run under `cmd /C` instead of failing to spawn. Each entry supports `env` (edited as `KEY=VALUE` lines in the UI), and editing preserves the existing enablement and trust. Skills are scanned from `~/.maestro/skills` and `<workspace>/.maestro/skills` (project shadows global); new skills can be created in either scope and existing ones deleted from the UI.

Slash commands are a composer-level palette. Typing `/` at the start of the prompt opens three groups — built-in actions, skills, and custom commands — with ↑/↓ to move, Enter/Tab to run, and Escape to dismiss for the current edit. Actions switch mode, create a session, stop the run, toggle the review panel, open settings/extensions/activity, or clear the input. Selecting a skill inserts a `load_skill` directive plus any text typed after the command. Custom commands are Markdown files in `~/.maestro/commands` and `<workspace>/.maestro/commands` (project shadows global) with optional frontmatter:

```markdown
---
name: review
description: 审查变更
argumentHint: <路径>
---
请审查 $ARGUMENTS，并给出风险清单。
```

Selecting one reads the body, replaces `$ARGUMENTS` with the text after the command (or appends it when the template has no placeholder), and writes the result into the composer for review before sending.

Subagent runs are first-class traces. Each `spawn_subagent` gets an id such as `sa-<run>-<n>`; start/finish are persisted in `subagent_traces` (v9), while streaming content, reasoning and child tool calls arrive as `subagent_*` events. Child tool calls never enter the parent tool timeline; they render inside the spawn row and in the right-panel “子智能体” tab, which shows prompt, reasoning, streamed answer, tool list, usage and terminal error. Clicking a spawn row opens that tab, and traces survive session switches and restarts (running traces are marked failed on restart). The final summary still returns to the parent model as the `spawn_subagent` tool result; in addition, later turns receive a bounded block (latest 6 reports, ≤16KB) so a follow-up can cite earlier findings. A child is capped at 600s; on timeout or step exhaustion it returns the findings collected so far, and the trace panel offers a per-child stop button.

On OpenCode gateways (`opencode.ai`), `x-opencode-session` routes and limits concurrent streams. The parent run uses `maestro-<sessionId>`; every child gets `<parent>-sub-<subagentId>` so concurrent research streams are not cut mid-flight. The default fan-out is 3 (`MAESTRO_MAX_SUBAGENTS=1..4`); a mid-stream network failure is retried once only when no tokens were emitted, and the underlying error is appended to the generic “网络连接已中断” message for diagnosis.

The model can call `ask_user` while a run is active. The kernel suspends that turn, emits a `user_question` event, and the UI opens a modal with 1–4 questions (2–4 options each, single or multi select) plus an explicit custom-answer field. Submitting resolves the suspension with structured answers that become the tool result (`【用户回答】…`); Skip sends an empty answer set (`用户跳过了提问`). Cancelling the run resolves the wait with an error. The finished question/answer payload is stored in the tool audit detail (≤8KB) so the tool row can be expanded after reload; it is separate from the binary plan/tool approvals.

Subagent drill-down reuses the main chat timeline: reasoning bursts, tool rows and streamed content are interleaved in event order and the panel follows the bottom with the same sticky-scroll contract. File targets in a child trace call the shared workspace opener, so the Explorer preview loads instead of staying on “正在读取文件内容…”. The default fan-out is 3 with a 600s per-child timeout and a 20-tool budget; both concurrency and timeout are env-tunable (`MAESTRO_MAX_SUBAGENTS`, `MAESTRO_SUBAGENT_TIMEOUT`).

Run metrics accumulate across model turns: each turn contributes its output, elapsed stream time and first-token time, while input/cache are summed as billed usage. `SessionStats` derives decode speed from the cumulative values and labels the duration as model-stream time; the token pill is explicitly “总 tokens”. When a run completes successfully, the task plan auto-completes any remaining pending/in-progress steps; failed or cancelled runs keep their state.

All feature dialogs now share `src/components/ui/dialog.tsx`. The adapter owns Portal, overlay, fixed centering, z-index and the `default`/`form`/`panel`/`palette` variants; feature classes only style content. This removes the class of bug where an overlay blurred the app while the dialog content rendered off-screen (`ask_user` was the last one).

The composer stats line follows DeepSeek Harness: Context (newest prompt / window), Input (billed = cache read + cache write + uncached), Output (provider-reported), Hit rate (cache read / billed input), tok/s (Σ output / Σ decode time over usage-reporting attempts) and TTFT (average first token). Clicking the line opens the breakdown (turns/steps, model-stream time, cache write, reasoning tokens, last-turn speed, context). Metrics are stored per run with `attempts`, `cache_write_tokens`, `reasoning_tokens` and `last_*` fields in `run_metrics` (schema v10), so the same numbers survive a restart; legacy rows fall back to `attempts=1` and the old totals.
