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
Rust measures HTTP/stream elapsed time and the first visible-delta latency (content, reasoning or tool-call argument deltas all count as the first delta, per the DSH `isTokenDelta` rule). Generation speed uses reported output tokens and the real decode window (first delta → stream end); fallback speed is labeled estimated, or remains unknown. SSE fragments are never counted as tokens. Metrics are stored in SQLite through its dedicated OS thread.

Images and videos are selected locally, limited to six files and 20 MiB each. Object URLs are revoked on removal/app unmount. Sending retains thumbnails as **not uploaded** message placeholders for the current app instance; no bytes, URLs or media metadata enter model IPC or persisted messages. An image-only request still requires a textual message. Media is not multimodal model inference.

Assistant Markdown disables raw HTML and automatic remote images. Code fences show language, syntax highlighting and a clipboard action; user bubbles expose Copy on hover/focus.

Dark/Light/System themes apply across the document, body-portaled menus and the code viewer (github-light/github-dark via the `maestro-theme` event); system listeners are cleaned up. Editor font/size also populate CSS code-font variables for Markdown and readonly previews. Language changes the basic frontend/navigation/settings labels, not model-generated text or backend error language.

## Validation

`pnpm.cmd build`, `pnpm.cmd test`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `pnpm.cmd screenshot`.
Local HTTP tests cover OpenAI usage and Anthropic text/thinking-budget/usage conversion without cloud credentials. Browser captures verify actual white Light theme, dynamic effort selection, popup hit-testing and unchanged workspace layouts. Actual cloud model availability/account limits still require your own valid configuration.

R1/context correction: **70 frontend tests**, production TypeScript/Vite build and Chromium screenshot suite passed. The added coverage exercises aliases, provider-specific strengths, R1 local deployment metadata, Settings persistence and storage-failure rollback. Reproduction screenshots: `artifacts/ui/maestro-r1-context.png`, `maestro-thinking-strength.png`, `maestro-model-capabilities.png`. They use isolated frontend preview data, not cloud inference.



## Timeline, tool rows and stop

- Reasoning bursts, tool calls and assistant text share one ordered timeline. Consecutive reasoning/tool items form an execution phase (`已工作 X 秒`); assistant text is rendered at the exact point it arrived instead of being appended after the whole turn.
- An execution phase starts expanded while the run is active and collapses when the phase ends or the run reaches a terminal state. Users can toggle it manually; manual state is not overridden.
- Tool and reasoning rows expose a right-side chevron with `aria-expanded`. Finished tool rows are intentionally quiet (gray text/icons) and only brighten on hover; `已执行` is not rendered, while failed/awaiting calls keep a status label.
- Clicking a file target opens the right-side Explorer and loads the file in the @pierre/diffs code viewer (virtualized text mount, shiki-wasm highlighting in a worker pool — the ZCode `CodeViewer` recipe). Kernel events travel one `kernel-event` channel; the sink batches through an unbounded queue and merges adjacent same-key token events (content/reasoning per run, subagent tokens per child+kind) inside a 32ms window so streaming bursts cannot starve invoke responses. Historical runs use the same `onOpenFile` path as live runs.
- Stop sends `abort_chat` and immediately seals the visible reasoning/tool state; there is no custom "stopping" pill. The final cancelled state is driven by the backend terminal event. The Rust kernel keeps a `cancelled_run` flag and a 250ms stream poll so cancellation does not depend on a single watch notification.

Composer stats follow the DeepSeek Harness pattern: two centered pills below the composer, provider-reported usage when available, and an explicit `est.` marker for live output estimates. Cache hit is cache-read tokens over billed prompt input; output tokens are excluded, and a missing provider cache bucket shows `—` rather than an invented value.

A run is capped at 64 model→tool→model rounds by default (`MAESTRO_MAX_TOOL_ROUNDS`, minimum 8). When the cap is reached, the next turn is tool-free and explicitly asks the user to send “继续” to start a new run; this prevents silent partial completion.

Code mode treats workspace writes as normal work; only a conservative dangerous-command list requires approval. Read-only tool calls in one batch run concurrently, while writes and commands stay serial. `spawn_subagent` runs a read-only research child loop and returns its summary to the parent model.

Subagent definitions are resolved from three sources with `builtin < global < project` precedence: built-ins (`explore`, `general`), `~/.maestro/agents/*.md`, and `<workspace>/.maestro/agents/*.md`. The settings panel lists the merged view with a source badge, locks the name of an existing definition, and lets a built-in be overridden by a global file (deleting that file restores the built-in). The `spawn_subagent` enum is generated from the same merged list, so a definition saved in Settings is selectable by the model without editing code; `hidden: true` and `mode` other than `subagent` stay out of the enum.

The child loop never inherits the parent's authorization: its tool calls are filtered against the tools offered to that agent and executed under the Plan gate, so `write_file`/`apply_patch`/`run_command` are denied even when the parent run is in Code or Auto. A consequence is that read-only child calls do not suspend for per-call approval in Confirm mode; the parent's approval slot is not consumed by research work.

Model reasoning levels come from the models.dev catalog (`reasoning_options[type=effort].values`); the model config dialog defaults to smart config and only shows the composer slider when more than one level is resolved. Advanced config can override input types, capabilities, ordered levels, and the reasoning mapping JSON.

Provider requests retry before the first token using the opencode policy: 5 attempts, 2s initial delay, x2 backoff, 25% jitter, 30s cap, honoring Retry-After; 401/403/404/400 are not retried, and a stream that already emitted tokens is not replayed.

Mid-stream transport breaks (gateway close, corrupt chunks) retry the whole request up to 3 times with backoff — but only before the first visible content delta lands (reasoning and tool-call-argument deltas do not block the retry: they are billed output, but regenerating them only duplicates text inside the collapsed thinking block rather than the visible answer). On retry the parser resets its per-attempt state and an internal `Retry` marker rewinds the attempt's timing/metrics (TTFT, decode window, surface estimate) so the failed attempt never enters the statistics — matching DSH's per-attempt accounting. A break after content has landed fails fast, keeps the partial text, and tells the user to retry the turn.

Composer tok/s uses provider-reported output tokens and provider timing only: `Σ decodeTokens / Σ decodeMs` across the session's runs, where the decode window of a request runs from its first visible delta (content, reasoning or tool-call arguments) to the end of its stream. Only requests that report both a first delta and output usage contribute to the pair; reasoning tokens are already inside the provider's output count and are never added again, tool execution time is excluded, and cancelled requests contribute nothing. It never derives the durable speed from estimated text; when usage is missing the pill shows `—`. Two DSH live semantics keep the line moving mid-request: (1) `liveSurfaceTokens` — a text-density estimate of the in-flight request's streamed surface (content+reasoning+tool-call arguments — arguments are billed output too; CJK=1 / other÷4, reset whenever the provider reports an output usage sample) rides every live metrics tick, so the token total grows during streaming and a marked `≈ tok/s` in-flight speed appears while the run streams; it never persists. (2) usage-survives-failure: a stream that ends without a Done still folds whatever usage already arrived into the cumulative buckets (cancelled runs contribute nothing) — but the failed step never touches attempts/TTFT/decode/llmMs, matching the Harness split where tokenUsage keeps early samples while sessionStats only folds closed messages. Repeated usage samples replace earlier values per field (Anthropic `message_start` output seed is overwritten by `message_delta`).
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

Run metrics accumulate across model turns — including subagent LLM calls, which fold into the parent run as real model steps (usage, decode pair, TTFT, llmMs) while they run: each turn contributes its output, elapsed stream time and first-token time, while input/cache are summed as billed usage. Decode windows (`decode_ms`) and their paired tokens (`decode_tokens`) accumulate separately from plain usage so a request that produced no visible delta or reported no usage cannot pollute the speed ratio; tool wall time lands in `tool_ms`, never in decode. `SessionStats` derives the official 输出速度 (TPS) from the cumulative pair and labels the duration as 模型用时 (model time); while a run streams, the stat line also shows a marked `≈` in-flight TPS computed from `liveSurfaceTokens` over the current request's decode window (lastElapsed − lastFirstToken), and the popover adds 本轮速度（估）. When a run completes successfully, the task plan auto-completes any remaining pending/in-progress steps; failed or cancelled runs keep their state.

All feature dialogs now share `src/components/ui/dialog.tsx`. The adapter owns Portal, overlay, fixed centering, z-index and the `default`/`form`/`panel`/`palette` variants; feature classes only style content. This removes the class of bug where an overlay blurred the app while the dialog content rendered off-screen (`ask_user` was the last one).

The composer stats line follows the DeepSeek Harness StatsLine layout: `{turns} 轮 {steps} 步 · {tps} tok/s · {total} tok · 缓存命中 {pct}% · {ctx}%`. Steps count completed LLM requests (`attempts`, the DSH step/end count) — not tool calls. tok/s is the official 输出速度 (TPS): `Σ decodeTokens / Σ decodeMs` over requests pairing a first delta with output usage; TTFT averages `Σ first-delta latency / steps that saw one`; cache hit is an integer percent of billed input. Providers only report token usage when a request ends (Anthropic also sends input at `message_start`), so tok/s/token figures update at request boundaries — the relay emits a metrics event the instant a `Usage` chunk or the first delta arrives, and every `Done` persists, matching DSH's step-close semantics; the output estimate keeps the total live mid-stream and is flagged `est.`. Clicking the line opens the breakdown in DSH order: turns/steps, 模型用时, 工具调用用时, 首 token 平均 (TTFT), 输出速度 (TPS), then the token detail (billed input, cache read/write, uncached, output, reasoning, cache hit, context). Metrics are stored per run with `attempts`, `cache_write_tokens`, `reasoning_tokens`, `last_*` fields and the `ttft_steps`/`decode_ms`/`decode_tokens`/`tool_ms` pair columns in `run_metrics` (schema v15), so the same numbers survive a restart; legacy rows fall back to the `elapsed - firstToken` approximation without backfilling.

Tool approvals are no longer binary. A suspended batch offers Allow once, Always allow and Deny. Always allow writes session-scoped rules derived from the suspended calls: `run_command` stores either the command prefix (matches the same binary) or the exact command, while `write_file`/`apply_patch` store the target path and MCP tools store a tool-level rule. Later `RequireApproval` calls that match a rule execute without a prompt and are audited as `auto-approved`. Deny can carry optional feedback; the tool result becomes `用户拒绝了该操作：<feedback>` so the model can adjust. Rules are persisted in SQLite (`approval_rules`, schema v18) and survive restarts; `list_approval_rules`/`revoke_approval_rule` enumerate and revoke them. Plan-mode denies are never bypassed by a rule.

Conversation assembly replays every terminal run in the same session, not just completed ones. A cancelled or failed run contributes its prompt, a marker explaining that the output is partial, the partial text, and any error; a completed run with no text contributes a placeholder. Each replayed run also gets a compact system summary of its last 12 tool actions (`write_file · path`, `run_command · command`) so the model remembers what it actually did instead of re-reading the workspace. The byte budget is preserved: prompt and tool summary are kept first, and an oversized output is truncated to its head and tail rather than dropping the whole turn. New sessions still start with no cross-session memory.

Context compaction follows ZCode's `context_compaction` separator. Auto compaction runs before a model turn when the estimated messages exceed 80% of the model context window; `/compact` triggers the same flow manually. Older messages are summarized with the same provider (falling back to a deterministic excerpt when the summary call fails), the summary is persisted in `session_compactions` (schema v11), and `conversation` replays the summary plus only the runs after `through_run_id`. The UI renders a separator with trigger, status, pre/post token counts and an expandable summary; failures/interruptions keep their status. A provider context-length error forces one compaction and retry. The Context chip/ring now uses the newest prompt (`lastInputTokens`) rather than cumulative billed input, and shows `—` when that value is unavailable. Only main-conversation requests update it — subagent (and any sidecar) calls fold into token/step totals but never overwrite the parent context meter (ZCode `shouldUseModelUsageForContext`), so a small child prompt cannot drop the ring to 0%. Percentages keep one decimal (`Intl` percent format), so 2K/1M reads 0.2% rather than a misleading 0%.

Context usage follows ZCode's `getRenderableTaskUsage`: the ring and Context chip only render when the latest provider-reported prompt and the model window are finite and positive. Until usage arrives they are hidden (no 0%, no “unavailable”, no estimate); cumulative billed input is never used as context. Cache hit is shown in the stats line only at or above 78%, matching ZCode's production threshold, while the detail popover keeps the exact value.

Run-internal tool messages are persisted in `run_messages` (schema v12): each `assistant` tool-call message and its `tool` result is stored with a stable sequence number, so a later run in the same session replays the actual tool transcript instead of only prompt + final output + a tool summary. Tool results are truncated to 64 KiB when stored; replay stays within the 131072-byte conversation budget and falls back to the summary for legacy runs without messages or when a run exceeds the budget. Compaction boundaries still skip covered runs. This is what keeps the context indicator from dropping between consecutive runs (for example 7% to 4%).

Reasoning and content deltas are coalesced in the Rust relay before reaching the UI, matching ZCode's `agent_thought_chunk` / `agent_message_chunk` granularity: consecutive deltas accumulate and flush on 32ms, 512 bytes, or before any non-reasoning event (content/tool/usage/done); a timer also flushes while the stream is idle. The frontend therefore applies one timeline update per chunk instead of one per token, avoiding the previous O(n²) array copy and per-token React render. Cancelling drops the unflushed tail by design.

The right-panel “+” dropdown is controlled: clicking outside the menu or trigger, clicking the menu padding, or selecting an already-open view closes it. Already-open entries are no longer disabled; selecting them is a no-op and closes the menu.
