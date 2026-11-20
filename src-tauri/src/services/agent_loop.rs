//! Agent loop (ReAct): think → call tools → observe → answer. The LLM
//! adapter streams normalized `StreamEvent`s over a channel; this service
//! relays them to the kernel as typed domain events, executes tool calls via
//! the kernel's authorized path, and feeds results back for the next turn.
use std::{sync::{atomic::{AtomicU64, Ordering}, Arc}, time::{Duration, Instant}};

use futures_util::future::join_all;

use tokio::sync::{mpsc, watch};

use crate::{
    adapters::llm::{ChatConfig, LlmProvider, StreamEvent, TurnOutcome},
    domain::{
        AppResult, ChatMessage, ContextCompaction, ExtensionKind, Mode, RunMessage, RunMetrics,
        RunStatus, RunView, SessionDetail, SubagentStatus, SubagentTokenKind, SubagentView,
        ToolCallView, ToolPhase, ToolSpec,
    },
    services::{kernel::Kernel, tool_router::SubagentGrant},
};

/// Safety cap for one run; can be raised with MAESTRO_MAX_TOOL_ROUNDS.
fn max_tool_rounds() -> usize {
    std::env::var("MAESTRO_MAX_TOOL_ROUNDS").ok().and_then(|value| value.parse::<usize>().ok()).filter(|value| *value >= 8).unwrap_or(64)
}
const MAX_TOOL_RESULT_BYTES: usize = 65536;

/// 工具结果回传模型的上限截断：必须带提示——静默截断会让模型误以为拿到
/// 完整结果（与 exec 「省略 N 字节」、read_file 截断提示同一约定）。
fn bounded_result_text(text: &str) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_BYTES {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_TOOL_RESULT_BYTES).collect();
    out.push_str("…（工具结果已截断）");
    out
}
const METRICS_REPORT_INTERVAL: Duration = Duration::from_millis(200);
/// ZCode-style chunking: coalesce raw deltas before emitting to the UI.
const STREAM_CHUNK_INTERVAL: Duration = Duration::from_millis(32);
const STREAM_CHUNK_BYTES: usize = 512;

/// Scoped reads only reach the selected workspace. Tell the model up front so
/// it explains the boundary instead of burning turns probing outside paths.
const SANDBOX_HINT: &str = "如果任务需要工作区之外的文件（例如用户提到的其他仓库路径），不要尝试绕过沙盒读取；请明确告知用户该路径不在工作区，并请用户将其复制或移动到工作区内后再继续。";

/// Task-tracker discipline: the model must publish a plan before multi-step
/// work and refresh it after every finished step, otherwise the UI panel has
/// nothing to render.
const SUBAGENT_HINT: &str = "当任务需要同时分析多个独立文件/目录，或需要并行只读调研时，优先在同一轮调用多个 spawn_subagent（最多 3 个），并选择合适的 subagent_type：explore 用于代码调研，general 用于分析总结。单文件读取或简单顺序任务不要使用。子智能体只读，不能写文件或执行命令；把返回摘要作为证据。";

/// Fallback instruction when a user-defined agent has an empty prompt body.
const DEFAULT_SUBAGENT_PROMPT: &str = "你是只读研究子智能体。只使用提供的只读工具分析，不写文件、不执行命令；完成后用简洁中文给出结论、证据路径和风险。";

/// Efficiency budget: broad research otherwise reads every file until timeout.
fn subagent_budget_hint(tool_budget: usize) -> String {
    format!("效率要求：先列出最关键的文件清单，再批量读取；不要重复读取同一文件；最多使用 {tool_budget} 次工具调用；接近预算或时间上限时立即输出已有结论、证据路径与风险，不要继续尝试。")
}

/// A child that stops producing progress must not hold the parent turn forever.
/// The default lives in `domain::DEFAULT_SUBAGENT_TOOL_BUDGET`; definitions may
/// override it via `tool_budget`.
fn subagent_tool_budget(definition: &crate::domain::AgentDefinition) -> usize {
    definition.tool_budget.unwrap_or(crate::domain::DEFAULT_SUBAGENT_TOOL_BUDGET).clamp(1, crate::domain::MAX_SUBAGENT_TOOL_BUDGET) as usize
}

fn subagent_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("MAESTRO_SUBAGENT_TIMEOUT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (60..=1800).contains(value))
            .unwrap_or(600),
    )
}
/// Partial findings kept when a child times out or exhausts its step budget.
const SUBAGENT_PARTIAL_BYTES: usize = 16384;

const HISTORY_HINT: &str = "历史中被中断的轮次只包含部分输出；它们是已完成的工作记录，不要重复执行，除非用户要求。";

const TASK_PLAN_HINT: &str = "在执行涉及多个文件或步骤的复杂任务前，必须先调用 update_plan 列出计划（每个步骤包含 id、description、status；初始为 pending，当前要做的步骤为 in_progress）。在后续执行中，每完成一步，必须重新调用 update_plan 提交完整步骤列表并更新各步状态；全部完成时所有步骤都应为 completed。简单的单步任务可以跳过。";

/// How many bytes of prior turns may be replayed, derived from the model's
/// context window instead of a fixed 128 KiB: a 1M-token model should not be
/// handed the same history as a 128k one (that silently truncated the
/// conversation and made the context ring drop between turns).
pub fn replay_budget_bytes(context_window: Option<u64>) -> usize {
    const DEFAULT_WINDOW_TOKENS: u64 = 128_000;
    // ~4 bytes per token, 60% of the window reserved for replayed history.
    let window = context_window.filter(|value| *value > 0).unwrap_or(DEFAULT_WINDOW_TOKENS);
    let bytes = (window as f64 * 0.6 * 4.0) as u64;
    bytes.clamp(65_536, 8 * 1024 * 1024) as usize
}

/// Conversation assembly: system policy for the run's mode plus prior
/// completed turns of the same session within a window-derived byte budget.
pub fn conversation(detail: &SessionDetail, run: &RunView, context_window: Option<u64>) -> Vec<ChatMessage> {
    let policy = match run.mode {
        Mode::Plan => "你处于只读计划模式。仅可使用列表中提供的只读工具进行分析；写操作与命令执行不会出现在工具列表中，也不得声称已应用变更。",
        Mode::Code => "你处于 Code 模式。读操作和写文件/补丁直接执行；只有危险命令会挂起等待批准，安全命令直接执行——以工具结果为准，不得声称已应用未成功的修改。",
        Mode::Confirm => "你处于 Confirm 模式。每个工具调用（含只读）都会挂起等待用户逐项确认后才执行；以工具结果为准，不得声称已执行未经确认的操作。",
        Mode::Execute => "你处于历史 execute 模式（遗留记录）。仅可使用实际开放且授权的能力。",
        Mode::Auto => "你处于 Auto 策略。已开放的工具调用会直接执行；以工具结果为准，不得声称拥有未提供的能力。",
    };
    let mut messages = vec![ChatMessage::text(
        "system",
        format!("{policy}\n{SANDBOX_HINT}\n{SUBAGENT_HINT}\n{TASK_PLAN_HINT}\n{HISTORY_HINT}"),
    )];
    let latest = detail
        .compactions
        .iter()
        .rev()
        .find(|compaction| compaction.status == "completed");
    let boundary = latest.map(|compaction| compaction.through_run_id).unwrap_or(0);
    if let Some(compaction) = latest {
        messages.push(ChatMessage::text(
            "system",
            format!("[上下文压缩摘要]
{}", compaction.summary),
        ));
    }
    let prior: Vec<_> = detail
        .runs
        .iter()
        .filter(|item| {
            item.id > boundary
                && item.id < run.id
                && item.session_id == run.session_id
                && item.status != RunStatus::Running
        })
        .collect();
    let mut budget = replay_budget_bytes(context_window);
    let mut included = Vec::new();
    for item in prior.into_iter().rev().take(32) {
        let summary = tool_summary(detail, item.id);
        let mut output = item.output.clone();
        let mut run_messages: Vec<RunMessage> = detail
            .run_messages
            .iter()
            .filter(|message| message.run_id == item.id)
            .cloned()
            .collect();
        let mut message_bytes: usize = run_messages.iter().map(|message| message.content.len()).sum();
        let mut size = item.prompt.len() + output.len() + summary.len() + message_bytes;
        if size > budget {
            output = truncate_middle(&output, 4096);
            size = item.prompt.len() + output.len() + summary.len() + message_bytes;
        }
        if size > budget {
            // Compress tool bodies before dropping the run: clearing the whole
            // list loses the conversation shape the model needs to continue.
            for message in &mut run_messages {
                if message.content.len() > 4096 {
                    message.content = truncate_middle(&message.content, 4096);
                }
            }
            message_bytes = run_messages.iter().map(|message| message.content.len()).sum();
            size = item.prompt.len() + output.len() + summary.len() + message_bytes;
        }
        if size > budget {
            run_messages.clear();
            size = item.prompt.len() + output.len() + summary.len();
        }
        if size > budget {
            output.clear();
            size = item.prompt.len() + summary.len();
        }
        if size > budget {
            break;
        }
        budget -= size;
        included.push((item, output, summary, run_messages));
    }
    for (item, output, summary, run_messages) in included.into_iter().rev() {
        messages.push(ChatMessage::text("user", item.prompt.clone()));
        if !run_messages.is_empty() {
            for message in &run_messages {
                let tool_calls = message
                    .tool_calls
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok());
                messages.push(ChatMessage {
                    role: message.role.clone(),
                    content: message.content.clone(),
                    tool_calls,
                    tool_call_id: message.tool_call_id.clone(),
                    name: message.name.clone(),
                    images: None,
                    thinking: message.thinking.clone(),
                    is_error: message.is_error,
                });
            }
        }
        let mut body = match item.status {
            RunStatus::Completed => {
                if output.trim().is_empty() { "[上一轮没有最终文字输出]".to_string() } else { output }
            }
            RunStatus::Cancelled => format!(
                "[上一轮被用户中断（cancelled），以下为已产生的部分输出，不是最终答案]\n{}",
                if output.trim().is_empty() { "（没有已产生的输出）" } else { output.as_str() }
            ),
            RunStatus::Failed => format!(
                "[上一轮执行失败（failed），以下为已产生的部分输出，不是最终答案]\n{}",
                if output.trim().is_empty() { "（没有已产生的输出）" } else { output.as_str() }
            ),
            RunStatus::Running => continue,
        };
        if let Some(error) = item.error.as_deref().filter(|value| !value.trim().is_empty()) {
            body.push_str(&format!("\n[错误：{error}]"));
        }
        messages.push(ChatMessage::text("assistant", body));
        if run_messages.is_empty() && !summary.is_empty() {
            messages.push(ChatMessage::text(
                "system",
                format!("上一轮实际执行过的工具（供参考，不要当作新任务）：\n{summary}"),
            ));
        }
    }
    messages.push(ChatMessage::text("user", run.prompt.clone()));
    messages
}

fn tool_summary(detail: &SessionDetail, run_id: i64) -> String {
    let calls: Vec<&ToolCallView> = detail.tools.iter().filter(|call| call.run_id == run_id).collect();
    let start = calls.len().saturating_sub(12);
    calls[start..]
        .iter()
        .map(|call| {
            let text = call.detail.replace(['\n', '\r'], " ");
            format!("- {} · {}", call.tool, text.chars().take(120).collect::<String>())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let half = max / 2;
    format!(
        "{}\n…\n{}",
        chars[..half].iter().collect::<String>(),
        chars[chars.len() - half..].iter().collect::<String>()
    )
}

/// Same as `conversation`, plus the architecture contract (user-drawn rules
/// enforced by Rust) and a bounded block of earlier child reports so a
/// follow-up turn can still cite what subagents found in previous runs.
pub fn conversation_with_reports(
    detail: &SessionDetail,
    run: &RunView,
    reports: &str,
    context_window: Option<u64>,
    architecture_brief: &str,
) -> Vec<ChatMessage> {
    let mut messages = conversation(detail, run, context_window);
    if !reports.trim().is_empty() {
        messages.insert(1, ChatMessage::text("system", reports));
    }
    if !architecture_brief.trim().is_empty() {
        messages.insert(1, ChatMessage::text("system", architecture_brief.to_string()));
    }
    messages
}

async fn flush_reasoning(kernel: &Kernel, run_id: i64, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    let chunk = std::mem::take(buffer);
    kernel.reasoning_token(run_id, chunk).await;
}

async fn flush_content(kernel: &Kernel, run_id: i64, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    let chunk = std::mem::take(buffer);
    kernel.token(run_id, chunk).await;
}

/// One provider turn with event relay. Returns when the provider's stream
/// terminates (Done → `TurnOutcome`) or the run is cancelled.
async fn stream_turn(
    kernel: &Arc<Kernel>,
    provider: &Arc<dyn LlmProvider>,
    config: &ChatConfig,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    cancel: &mut watch::Receiver<bool>,
    run_id: i64,
) -> AppResult<TurnOutcome> {
    let session_id = kernel
        .snapshot()
        .await
        .active_run
        .as_ref()
        .filter(|run| run.id == run_id)
        .map(|run| run.session_id)
        .ok_or("Run no longer active")?;
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();
    let provider_name = config.provider.name().to_string();
    let model_name = config.model.clone();
    let relay = {
        let kernel = kernel.clone();
        let mut relay_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut started = Instant::now();
            let mut reported = Instant::now();
            let mut first_token_ms = None;
            let mut metrics = RunMetrics {
                run_id,
                session_id,
                provider: provider_name,
                model: model_name,
                input_tokens: None,
                output_tokens: None,
                cache_hit_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
                attempts: 0,
                last_input_tokens: None,
                last_output_tokens: None,
                last_elapsed_ms: 0,
                last_first_token_ms: None,
                elapsed_ms: 0,
                first_token_ms: None,
                ttft_steps: 0,
                decode_ms: 0,
                decode_tokens: 0,
                tool_ms: 0,
                live_surface_tokens: None,
            };
            // Drain relay: drop any buffered events the instant the run is
            // cancelled — this is what makes "stop" actually stop the stream.
            let mut reasoning_buf = String::new();
            let mut content_buf = String::new();
            let mut last_chunk = Instant::now();
            let mut saw_done = false;
            // DSH surfaceTokens 口径：当前请求已流式 surface 文本的估算 tokens
            // （content+reasoning，CJK=1/其他÷4）。provider 上报 output usage
            // 后计数清零——估算只覆盖最近一个 usage 样本之后的文本。
            let mut surface_cjk: u64 = 0;
            let mut surface_other: u64 = 0;
            let surface_est = |metrics: &mut RunMetrics, cjk: u64, other: u64| {
                let est = cjk + other.div_ceil(4);
                metrics.live_surface_tokens = (est > 0).then_some(est);
            };
            loop {
                let event = tokio::select! {
                    biased;
                    event = rx.recv() => event,
                    _ = relay_cancel.changed() => break,
                    _ = tokio::time::sleep(Duration::from_millis(8)) => {
                        if last_chunk.elapsed() >= STREAM_CHUNK_INTERVAL {
                            flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                            flush_content(&kernel, run_id, &mut content_buf).await;
                            last_chunk = Instant::now();
                        }
                        continue;
                    }
                };
                let Some(event) = event else {
                    flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                    flush_content(&kernel, run_id, &mut content_buf).await;
                    // DSH：usage 样本在请求失败时仍然保留（tokenUsage），但失败
                    // step 不进 sessionStats —— 无 Done 的终止只折叠 usage 桶，
                    // 不贡献 attempts/ttft/decode/llmMs。
                    if !saw_done {
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        if metrics.input_tokens.is_some()
                            || metrics.output_tokens.is_some()
                            || metrics.cache_hit_tokens.is_some()
                            || metrics.cache_write_tokens.is_some()
                            || metrics.reasoning_tokens.is_some()
                        {
                            let _ = kernel.report_usage_sample(metrics.clone()).await;
                        }
                    }
                    break;
                };
                // DSH 口径（dsh-llm isTokenDelta）：首个非空 text/reasoning/
                // tool-call delta 即首 token，三种 delta 共用同一打点与 tick。
                let is_delta = matches!(
                    event,
                    StreamEvent::Token(_) | StreamEvent::Reasoning(_) | StreamEvent::ToolCallDelta(_)
                );
                match event {
                    StreamEvent::Reasoning(token) => {
                        count_surface(&token, &mut surface_cjk, &mut surface_other);
                        reasoning_buf.push_str(&token);
                        if reasoning_buf.len() >= STREAM_CHUNK_BYTES || last_chunk.elapsed() >= STREAM_CHUNK_INTERVAL {
                            flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                            last_chunk = Instant::now();
                        }
                    }
                    StreamEvent::ToolCallDelta(args) => {
                        // 参数文本也是 provider 计费的输出，计入 surface 估算；
                        // tool-call delta 同样结束当前思考 burst。
                        count_surface(&args, &mut surface_cjk, &mut surface_other);
                        flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                    }
                    StreamEvent::Token(token) => {
                        count_surface(&token, &mut surface_cjk, &mut surface_other);
                        // A content delta always ends the current thought burst.
                        flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                        content_buf.push_str(&token);
                        if content_buf.len() >= STREAM_CHUNK_BYTES || last_chunk.elapsed() >= STREAM_CHUNK_INTERVAL {
                            flush_content(&kernel, run_id, &mut content_buf).await;
                            last_chunk = Instant::now();
                        }
                    }
                    StreamEvent::Usage { input, output, cache_hit, cache_write, reasoning } => {
                        flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                        flush_content(&kernel, run_id, &mut content_buf).await;
                        // DSH tokenUsage addReplacing：同一请求的后续 usage
                        // 样本替换前值（Anthropic message_start 带
                        // output_tokens:1 种子，最终值在 message_delta）。
                        if let Some(value) = input { metrics.input_tokens = Some(value); }
                        if let Some(value) = output { metrics.output_tokens = Some(value); }
                        if let Some(value) = cache_hit { metrics.cache_hit_tokens = Some(value); }
                        if let Some(value) = cache_write { metrics.cache_write_tokens = Some(value); }
                        if let Some(value) = reasoning { metrics.reasoning_tokens = Some(value); }
                        // Provider 的 output usage 样本已覆盖此前流式文本——
                        // 估算计数清零，只统计该样本之后的新文本。
                        if output.is_some() {
                            surface_cjk = 0;
                            surface_other = 0;
                        }
                        // Surface usage the moment the provider reports it
                        // (Anthropic sends input usage at message_start) instead
                        // of waiting for the next delta tick or Done.
                        surface_est(&mut metrics, surface_cjk, surface_other);
                        let _ = kernel.report_metrics(metrics.clone(), false).await;
                        reported = Instant::now();
                    }
                    StreamEvent::Retry => {
                        // DSH：失败 attempt 不进 sessionStats——新 attempt 重新
                        // 计时与估算；已流出的 reasoning 是真实产出，保留在
                        // 思考折叠段内不回撤。
                        started = Instant::now();
                        first_token_ms = None;
                        metrics.first_token_ms = None;
                        surface_cjk = 0;
                        surface_other = 0;
                        metrics.live_surface_tokens = None;
                    }
                    StreamEvent::Done => {
                        flush_reasoning(&kernel, run_id, &mut reasoning_buf).await;
                        flush_content(&kernel, run_id, &mut content_buf).await;
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        let _ = kernel.report_metrics(metrics.clone(), true).await;
                        saw_done = true;
                    }
                }
                if is_delta {
                    let now_ms = started.elapsed().as_millis() as u64;
                    if first_token_ms.is_none() {
                        first_token_ms = Some(now_ms);
                        metrics.first_token_ms = first_token_ms;
                        // Emit immediately so TTFT shows without waiting for the
                        // 200ms tick.
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        surface_est(&mut metrics, surface_cjk, surface_other);
                        let _ = kernel.report_metrics(metrics.clone(), false).await;
                        reported = Instant::now();
                    }
                    // Tick on every delta kind: a long reasoning burst must keep
                    // the stats line live instead of freezing until content.
                    if reported.elapsed() >= METRICS_REPORT_INTERVAL {
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        surface_est(&mut metrics, surface_cjk, surface_other);
                        let _ = kernel.report_metrics(metrics.clone(), false).await;
                        reported = Instant::now();
                    }
                }
            }
        })
    };
    let outcome = provider.clone().stream(config, messages, tools, cancel, tx).await;
    let _ = relay.await;
    outcome
}

/// Read-only research subagent with its own persisted trace. It never writes
/// files or runs commands; the final text is returned to the parent model and
/// stored in the drill-down.
struct ChildOutcome {
    status: SubagentStatus,
    content: String,
    error: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    first_token_ms: Option<u64>,
    elapsed_ms: u64,
    /// Σ 每次 LLM 调用的 decode 窗口（首 delta → 末 delta），仅计入上报
    /// output usage 且流正常结束的调用 —— 与 DSH decodeTokens/decodeMs 同口径。
    decode_ms: u64,
    /// 与 decode_ms 成对的 output tokens。
    decode_tokens: u64,
    last_input_tokens: Option<u64>,
    tool_count: usize,
}

/// DSH surfaceTokens 同款文本密度估算：CJK（U+3400–U+9FFF，与前端
/// `estimateTokens` 的 `[㐀-鿿]` 同区间）每字 1 token，其余每 4 字符
/// 1 token。只用于 live 显示，永不进持久化。
fn count_surface(text: &str, cjk: &mut u64, other: &mut u64) {
    for ch in text.chars() {
        if ('\u{3400}'..='\u{9fff}').contains(&ch) {
            *cjk += 1;
        } else {
            *other += 1;
        }
    }
}

fn add_tokens(total: Option<u64>, extra: Option<u64>) -> Option<u64> {
    match (total, extra) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

/// One completed provider call's drained stream, kept separate so tool calls
/// and retries can't contaminate the per-call decode window.
#[derive(Default)]
struct SubagentCall {
    /// 子智能体启动 → 本调用首个有效 delta（content/reasoning/tool-call）。
    first_delta_ms: Option<u64>,
    /// 首 delta → 末 delta；仅当本调用上报了 output usage 才计入。
    decode_ms: u64,
    /// 与 decode_ms 成对的 output tokens（仅本调用有 decode 窗口时计入）。
    decode_tokens: u64,
    partial: String,
    input: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
    last_input: Option<u64>,
}

/// 并发排空子智能体的流事件：与 stream 同时进行才能拿到真实的
/// delta 到达时刻（事后 try_recv 排空会把整段流压成一个时刻）。
/// 每次调用同时把 metrics 折进父 run 的累计（父 run_id）——子智能体的
/// LLM 调用就是一次真实 model step，会话统计行应在子执行期间同样跳动。
/// 子调用排空的父会话归属：metrics 折叠到该 run/session。
struct SubagentDrainCtx {
    run_id: i64,
    session_id: i64,
    provider: String,
    model: String,
    subagent_id: String,
}

async fn drain_subagent_stream(
    kernel: Arc<Kernel>,
    ctx: SubagentDrainCtx,
    mut rx: mpsc::UnboundedReceiver<StreamEvent>,
    started: Instant,
) -> SubagentCall {
    let SubagentDrainCtx { run_id, session_id, provider, model, subagent_id } = ctx;
    let mut call = SubagentCall::default();
    let mut last_delta_ms = None;
    // 父 metrics 口径按“本调用”计时（与 stream_turn relay 同构）。
    let mut call_started = Instant::now();
    let mut reported = Instant::now();
    let mut call_first_ms: Option<u64> = None;
    let mut saw_done = false;
    let mut surface_cjk: u64 = 0;
    let mut surface_other: u64 = 0;
    let mut metrics = RunMetrics {
        run_id,
        session_id,
        provider,
        model,
        input_tokens: None,
        output_tokens: None,
        cache_hit_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
        attempts: 0,
        last_input_tokens: None,
        last_output_tokens: None,
        last_elapsed_ms: 0,
        last_first_token_ms: None,
        elapsed_ms: 0,
        first_token_ms: None,
        ttft_steps: 0,
        decode_ms: 0,
        decode_tokens: 0,
        tool_ms: 0,
        live_surface_tokens: None,
    };
    let surface_est = |m: &mut RunMetrics, cjk: u64, other: u64| {
        let est = cjk + other.div_ceil(4);
        m.live_surface_tokens = (est > 0).then_some(est);
    };
    while let Some(event) = rx.recv().await {
        // DSH isTokenDelta：text/reasoning/tool-call delta 都算有效增量。
        let is_delta = matches!(
            event,
            StreamEvent::Token(_) | StreamEvent::Reasoning(_) | StreamEvent::ToolCallDelta(_)
        );
        if is_delta {
            let at = started.elapsed().as_millis() as u64;
            if call.first_delta_ms.is_none() {
                call.first_delta_ms = Some(at);
            }
            last_delta_ms = Some(at);
            let call_at = call_started.elapsed().as_millis() as u64;
            if call_first_ms.is_none() {
                call_first_ms = Some(call_at);
                metrics.first_token_ms = call_first_ms;
                metrics.elapsed_ms = call_at;
                surface_est(&mut metrics, surface_cjk, surface_other);
                let _ = kernel.report_subagent_metrics(metrics.clone(), false).await;
                reported = Instant::now();
            }
        }
        match event {
            StreamEvent::Token(token) => {
                count_surface(&token, &mut surface_cjk, &mut surface_other);
                call.partial.push_str(&token);
                kernel.subagent_token(run_id, &subagent_id, SubagentTokenKind::Content, &token).await;
            }
            StreamEvent::Reasoning(token) => {
                count_surface(&token, &mut surface_cjk, &mut surface_other);
                kernel.subagent_token(run_id, &subagent_id, SubagentTokenKind::Reasoning, &token).await;
            }
            StreamEvent::ToolCallDelta(args) => {
                // 参数文本计入 surface 估算（provider 计费输出的一部分）。
                count_surface(&args, &mut surface_cjk, &mut surface_other);
            }
            StreamEvent::Usage { input, output, cache_hit, cache_write, reasoning } => {
                // DSH addReplacing：同调用的后续 usage 样本替换前值。
                if let Some(value) = input {
                    metrics.input_tokens = Some(value);
                    call.input = Some(value);
                    call.last_input = Some(value);
                }
                if let Some(value) = output {
                    metrics.output_tokens = Some(value);
                    call.output = Some(value);
                    // output 样本已覆盖此前流式文本，估算计数清零。
                    surface_cjk = 0;
                    surface_other = 0;
                }
                if let Some(value) = cache_hit {
                    metrics.cache_hit_tokens = Some(value);
                }
                if let Some(value) = cache_write {
                    metrics.cache_write_tokens = Some(value);
                }
                if let Some(value) = reasoning {
                    metrics.reasoning_tokens = Some(value);
                    call.reasoning = Some(value);
                }
                metrics.elapsed_ms = call_started.elapsed().as_millis() as u64;
                surface_est(&mut metrics, surface_cjk, surface_other);
                let _ = kernel.report_subagent_metrics(metrics.clone(), false).await;
                reported = Instant::now();
            }
            StreamEvent::Retry => {
                // 与主 relay 同口径：新 attempt 重新计时，decode 窗口与
                // surface 估算只覆盖成功 attempt。
                call_started = Instant::now();
                call_first_ms = None;
                metrics.first_token_ms = None;
                call.first_delta_ms = None;
                last_delta_ms = None;
                surface_cjk = 0;
                surface_other = 0;
                metrics.live_surface_tokens = None;
            }
            StreamEvent::Done => {
                // 子调用自身口径：decode 窗口 = 首个有效 delta → 末个 delta；
                // 只在与 output usage 成对时计入（DSH：无 usage 不进 decode）。
                if call.first_delta_ms.is_some() && call.output.is_some() {
                    if let (Some(first), Some(last)) = (call.first_delta_ms, last_delta_ms) {
                        call.decode_ms = last.saturating_sub(first);
                        call.decode_tokens = call.output.unwrap_or(0);
                    }
                }
                // 父会话口径：子调用整对 persist —— 一次真实 model step。
                metrics.elapsed_ms = call_started.elapsed().as_millis() as u64;
                let _ = kernel.report_subagent_metrics(metrics.clone(), true).await;
                saw_done = true;
            }
        }
        if is_delta && reported.elapsed() >= METRICS_REPORT_INTERVAL {
            metrics.elapsed_ms = call_started.elapsed().as_millis() as u64;
            surface_est(&mut metrics, surface_cjk, surface_other);
            let _ = kernel.report_subagent_metrics(metrics.clone(), false).await;
            reported = Instant::now();
        }
    }
    // 无 Done 的终止（断流/超时/取消）：只保留已到达的 usage 样本。
    if !saw_done {
        metrics.elapsed_ms = call_started.elapsed().as_millis() as u64;
        if metrics.input_tokens.is_some()
            || metrics.output_tokens.is_some()
            || metrics.cache_hit_tokens.is_some()
            || metrics.cache_write_tokens.is_some()
            || metrics.reasoning_tokens.is_some()
        {
            let _ = kernel.report_subagent_usage_sample(metrics.clone()).await;
        }
    }
    call
}

fn next_subagent_id(run_id: i64) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("sa-{run_id}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// OpenCode gateways route by `x-opencode-session`; a child must not share the
/// parent's id or concurrent streams get cut mid-flight.
fn child_session_id(parent: Option<&str>, subagent_id: &str) -> String {
    format!("{}-sub-{subagent_id}", parent.unwrap_or("maestro"))
}

fn max_parallel_subagents() -> usize {
    std::env::var("MAESTRO_MAX_SUBAGENTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=4).contains(value))
        .unwrap_or(2)
}

/// 装配契约段：拼进子系统提示——必选/可选条目 + 必选 Skill 正文预注入。
/// 与父级 architecture_brief 同口径：正文已给，但工具调用仍必须真实发生。
fn subagent_assembly_prompt(grants: &[SubagentGrant]) -> String {
    if grants.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n【架构装配】架构沙盘为本子智能体装配了扩展能力。必选条目必须由你真实调用——未调用完前不要输出最终结论。\n",
    );
    for grant in grants {
        let binding = &grant.binding;
        if let Some(missing) = &binding.missing {
            out.push_str(&format!("- {missing}：该能力当前不可用，照常继续。\n"));
            continue;
        }
        let level = if binding.required { "必选" } else { "可选建议" };
        let params = if binding.params.is_empty() {
            String::new()
        } else {
            let pairs = binding
                .params
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("，");
            format!("（参数提示：{pairs}）")
        };
        match binding.kind {
            ExtensionKind::Skill => out.push_str(&format!(
                "- {level}：调用 load_skill {}（正文已在下方预注入，调用本身仍必须真实发生）。{params}\n",
                binding.target
            )),
            ExtensionKind::Mcp => {
                let part = binding
                    .tool
                    .as_deref()
                    .map(|tool| format!("mcp__{}__{}", binding.target, tool))
                    .unwrap_or_else(|| format!("MCP 服务 {} 的任意工具", binding.target));
                out.push_str(&format!("- {level}：调用 {part}。{params}\n"));
            }
            ExtensionKind::Plugin => {}
        }
    }
    for grant in grants {
        if let (true, Some(body)) = (grant.binding.required, &grant.body) {
            out.push_str(&format!(
                "\n<skill_preload name=\"{}\">\n{body}\n</skill_preload>\n",
                grant.binding.target
            ));
        }
    }
    out
}

/// 必选调用的显示名（提醒/违例行共用）。
fn grant_label(grant: &SubagentGrant) -> String {
    let binding = &grant.binding;
    match binding.kind {
        ExtensionKind::Skill => format!("load_skill {}", binding.target),
        ExtensionKind::Mcp => binding
            .tool
            .as_deref()
            .map(|tool| format!("mcp__{}__{}", binding.target, tool))
            .unwrap_or_else(|| format!("MCP 服务 {} 的任意工具", binding.target)),
        ExtensionKind::Plugin => format!("插件 {}", binding.target),
    }
}

fn pending_grants_reminder(pending: &[SubagentGrant]) -> String {
    format!(
        "【架构装配】尚未完成的必选调用：{}。请先真实调用后再继续/收尾。",
        pending.iter().map(grant_label).collect::<Vec<_>>().join("、")
    )
}

/// 一次成功的子工具调用是否核销该必选绑定。
fn grant_satisfied(grant: &SubagentGrant, name: &str, arguments_json: &str) -> bool {
    let binding = &grant.binding;
    match binding.kind {
        ExtensionKind::Skill => {
            name == "load_skill"
                && serde_json::from_str::<serde_json::Value>(arguments_json)
                    .ok()
                    .and_then(|value| value.get("name").and_then(|v| v.as_str()).map(str::to_string))
                    .as_deref()
                    == Some(binding.target.as_str())
        }
        ExtensionKind::Mcp => match &binding.tool {
            Some(tool) => name == format!("mcp__{}__{}", binding.target, tool),
            None => name.starts_with(&format!("mcp__{}__", binding.target)),
        },
        ExtensionKind::Plugin => true,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_subagent(
    kernel: Arc<Kernel>,
    config: &ChatConfig,
    session_id: i64,
    run_id: i64,
    subagent_id: String,
    arguments_json: &str,
    mut parent_cancel: watch::Receiver<bool>,
) -> AppResult<String> {
    let arguments: serde_json::Value =
        serde_json::from_str(arguments_json).unwrap_or_else(|_| serde_json::json!({}));
    let description = arguments.get("description").and_then(|v| v.as_str()).unwrap_or("研究子任务").trim().to_string();
    let subagent_type = arguments.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("explore").trim().to_string();
    let prompt = arguments.get("prompt").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if prompt.is_empty() || prompt.len() > 8192 {
        return Err("spawn_subagent 需要 1–8192 字节的 prompt".into());
    }
    let available = kernel.list_agents("all".into()).await.unwrap_or_default();
    let names = available.iter()
        .filter(|agent| !agent.hidden && agent.mode == "subagent")
        .map(|agent| agent.name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    let definition = available.into_iter().find(|agent| agent.name == subagent_type)
        .or_else(|| crate::domain::AgentDefinition::builtin(&subagent_type))
        .ok_or_else(|| format!("未知子智能体类型：{subagent_type}；可用：{names}"))?;
    if definition.hidden || definition.mode != "subagent" {
        return Err(format!("子智能体 {subagent_type} 未启用：hidden 或 mode 不是 subagent"));
    }
    let child_config = ChatConfig {
        api_key: config.api_key.clone(), base_url: config.base_url.clone(),
        model: definition.model.clone().unwrap_or_else(|| config.model.clone()),
        provider: config.provider, reasoning: config.reasoning.clone(),
        max_output: config.max_output,
        context_window: config.context_window,
        session: Some(child_session_id(config.session.as_deref(), &subagent_id)),
    };
    let provider = kernel.llm_provider(&child_config);
    let run_root = kernel.run_workspace_root(run_id).await;
    // 沙盘装配：Extension → Subagent 绑定给该类型子智能体授权扩展能力
    // （Skill 预注入 + mcp__/load_skill 进白名单），required 在子循环内强制真实调用。
    let grants = kernel.subagent_grants(run_root.as_deref(), &definition.name).await;
    // 空白名单 = 全放行（既有语义）；装配只在非空白名单上扩权，不得误收窄。
    let base_open = definition.tools.is_empty();
    let mut allowed = definition.tools.clone();
    for grant in &grants {
        if grant.binding.missing.is_some() {
            continue;
        }
        let entry = match grant.binding.kind {
            ExtensionKind::Skill => "load_skill".to_string(),
            // 无 tool 绑定 = 该 Server 任意工具：`mcp__server__` 前缀条目。
            ExtensionKind::Mcp => match &grant.binding.tool {
                Some(tool) => format!("mcp__{}__{}", grant.binding.target, tool),
                None => format!("mcp__{}__", grant.binding.target),
            },
            ExtensionKind::Plugin => continue,
        };
        if !allowed.contains(&entry) {
            allowed.push(entry);
        }
    }
    // `mcp__server__` 结尾的条目是 server 级授权，按前缀匹配该 Server 全部工具。
    let tool_allowed = |name: &str| {
        allowed.iter().any(|entry| entry == name || (entry.ends_with("__") && name.starts_with(entry.as_str())))
    };
    let tools: Vec<_> = kernel.agent_tools(Mode::Plan, run_root.as_deref()).await.into_iter()
        .filter(|tool| tool.name != "spawn_subagent" && (base_open || tool_allowed(&tool.name)))
        .collect();
    let allowed_names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let tool_budget = subagent_tool_budget(&definition);
    let system_prompt = format!("{}
{}{}", if definition.prompt.trim().is_empty() { DEFAULT_SUBAGENT_PROMPT.to_string() } else { definition.prompt.clone() }, subagent_budget_hint(tool_budget), subagent_assembly_prompt(&grants));
    // 必选装配：子循环内逐轮强制，全部真实调用后才允许收尾。
    let mut pending_grants: Vec<SubagentGrant> = grants
        .into_iter()
        .filter(|grant| grant.binding.required && grant.binding.missing.is_none())
        .collect();
    let mut messages = vec![ChatMessage::text("system", system_prompt), ChatMessage::text("user", prompt.clone())];

    kernel.start_subagent(SubagentView {
        id: subagent_id.clone(),
        run_id,
        session_id,
        agent: definition.name.clone(),
        description: description.clone(),
        prompt,
        model: Some(child_config.model.clone()),
        status: SubagentStatus::Running,
        summary: String::new(),
        error: None,
        content: String::new(),
        reasoning: String::new(),
        tool_count: 0,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens: None,
        first_token_ms: None,
        elapsed_ms: 0,
        decode_ms: 0,
        decode_tokens: 0,
        last_input_tokens: None,
        started_at: crate::domain::unix_now(),
        finished_at: None,
        tools: Vec::new(),
    }).await?;

    let (child_tx, mut cancel) = watch::channel(*parent_cancel.borrow());
    kernel.register_subagent_cancel(subagent_id.clone(), child_tx.clone());
    let forward = tokio::spawn(async move {
        if parent_cancel.changed().await.is_ok() {
            let _ = child_tx.send(true);
        }
    });

    let started = Instant::now();
    let mut outcome = async {
        let timeout = subagent_timeout();
        let deadline = Instant::now() + timeout;
        let mut notes = String::new();
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut reasoning_tokens = None;
        let mut first_token_ms = None;
        let mut elapsed_ms = 0u64;
        let mut decode_ms = 0u64;
        let mut decode_tokens = 0u64;
        let mut last_input_tokens = None;
        let mut tool_count = 0usize;
        let timeout_message = format!("子智能体超时（{}s）", timeout.as_secs());
        for _ in 0..definition.steps.unwrap_or(8) {
            if tool_count >= tool_budget {
                return ChildOutcome { status: SubagentStatus::Completed, content: notes, error: Some(format!("达到工具预算（{tool_budget} 次），已返回部分结论")), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            if *cancel.borrow() {
                return ChildOutcome { status: SubagentStatus::Cancelled, content: notes, error: Some("子智能体已取消".into()), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(timeout_message), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            let mut attempt = 0;
            let (outcome, text) = loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(timeout_message), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
                }
                let (tx, rx) = mpsc::unbounded_channel();
                // 与 stream 并发排空：delta 时刻是真实到达时间，usage 逐次累加。
                let consumer = tokio::spawn(drain_subagent_stream(
                    kernel.clone(),
                    SubagentDrainCtx {
                        run_id,
                        session_id,
                        provider: child_config.provider.name().to_string(),
                        model: child_config.model.clone(),
                        subagent_id: subagent_id.clone(),
                    },
                    rx,
                    started,
                ));
                let streamed = tokio::time::timeout(
                    remaining,
                    provider.clone().stream(&child_config, &messages, &tools, &mut cancel, tx),
                ).await;
                let drained = consumer.await.unwrap_or_default();
                if first_token_ms.is_none() {
                    first_token_ms = drained.first_delta_ms;
                }
                decode_ms += drained.decode_ms;
                decode_tokens += drained.decode_tokens;
                input_tokens = add_tokens(input_tokens, drained.input);
                output_tokens = add_tokens(output_tokens, drained.output);
                reasoning_tokens = add_tokens(reasoning_tokens, drained.reasoning);
                last_input_tokens = drained.last_input.or(last_input_tokens);
                let partial = drained.partial;
                match streamed {
                    Ok(Ok(outcome)) => break (outcome, partial),
                    Ok(Err(error)) => {
                        let retryable = attempt == 0
                            && partial.trim().is_empty()
                            && error.contains("网络连接已中断")
                            && !*cancel.borrow();
                        if retryable {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                        if !partial.trim().is_empty() {
                            notes.push_str(&partial);
                        }
                        return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(error), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
                    }
                    Err(_) => {
                        if !partial.trim().is_empty() {
                            notes.push_str(&partial);
                        }
                        return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(timeout_message), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
                    }
                }
            };
            if !outcome.done {
                return ChildOutcome { status: SubagentStatus::Cancelled, content: notes, error: Some("子智能体已取消".into()), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            if !text.trim().is_empty() {
                notes.push_str(&text);
                if notes.len() > SUBAGENT_PARTIAL_BYTES {
                    notes = notes.chars().take(SUBAGENT_PARTIAL_BYTES).collect();
                }
            }
            if outcome.tool_calls.is_empty() {
                // 必选装配未清：不收尾，附装配提醒续跑（步数上限兜底）——
                // 与父级「正文已给但调用必须真实发生」同一口径。
                if !pending_grants.is_empty() {
                    messages.push(ChatMessage::text("user", pending_grants_reminder(&pending_grants)));
                    continue;
                }
                return ChildOutcome { status: SubagentStatus::Completed, content: text, error: None, input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: text,
                tool_calls: Some(outcome.tool_calls.clone()),
                tool_call_id: None,
                name: None,
                images: None,
                thinking: outcome.thinking.clone(),
                is_error: false,
            });
            let mut results: Vec<Option<AppResult<String>>> = vec![None; outcome.tool_calls.len()];
            let mut runnable: Vec<(String, String)> = Vec::new();
            let mut runnable_indices: Vec<usize> = Vec::new();
            for (index, call) in outcome.tool_calls.iter().enumerate() {
                if allowed_names.iter().any(|name| name == &call.name) {
                    runnable.push((call.name.clone(), call.arguments.clone()));
                    runnable_indices.push(index);
                } else {
                    results[index] = Some(Err(format!("子智能体不允许调用工具：{}", call.name)));
                }
            }
            tool_count += runnable.len();
            if !runnable.is_empty() {
                // Plan mode pins the gate to read-only regardless of the parent run's mode.
                let executed = kernel.execute_subagent_tool_calls(run_id, &subagent_id, &runnable, Some(Mode::Plan)).await;
                for ((index, result), (call_name, call_args)) in runnable_indices.into_iter().zip(executed).zip(&runnable) {
                    // 装配必选核销：仅成功调用算数（与会话历史判定同口径）。
                    if result.is_ok() {
                        pending_grants.retain(|grant| !grant_satisfied(grant, call_name, call_args));
                    }
                    results[index] = Some(result);
                }
            }
            for (call, result) in outcome.tool_calls.iter().zip(results) {
                let (text, is_error) = match result.unwrap_or_else(|| Err("missing tool result".into())) {
                    Ok(value) => (bounded_result_text(&value), false),
                    Err(error) => (format!("子智能体工具失败：{error}"), true),
                };
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: text,
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.name.clone()),
                    tool_calls: None,
                    images: None,
                    thinking: None,
                    is_error,
                });
            }
            // 必选装配逐轮跟进：只要还没真实调用完，每一轮都提醒——
            // 直到核销或步数/预算耗尽（耗尽时在结果尾部标注违例）。
            if !pending_grants.is_empty() {
                messages.push(ChatMessage::text("user", pending_grants_reminder(&pending_grants)));
            }
        }
        elapsed_ms = started.elapsed().as_millis() as u64;
        ChildOutcome {
            status: SubagentStatus::Completed,
            content: notes,
            error: Some("达到工具轮次上限，已返回部分结论".into()),
            input_tokens,
            output_tokens,
            reasoning_tokens,
            first_token_ms,
            elapsed_ms,
            decode_ms,
            decode_tokens,
            last_input_tokens,
            tool_count,
        }
    }.await;

    forward.abort();
    kernel.unregister_subagent_cancel(&subagent_id);
    // elapsed_ms 覆盖所有提前返回路径：总墙钟（含工具执行），decode 口径另列。
    outcome.elapsed_ms = started.elapsed().as_millis() as u64;
    let ChildOutcome { status, content, error, input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count } = outcome;
    let header = format!("【子智能体 {}｜{}｜{} 次工具】", definition.name, status.label(), tool_count);
    let mut summary = if content.trim().is_empty() { header.clone() } else { format!("{header}\n{}", content.trim()) };
    // 装配违例透传父模型：子任务已完成但沙盘必选扩展未被真实调用，
    // 结论可信度存疑——由父模型决定补调或重派。
    if !pending_grants.is_empty() {
        summary.push_str(&format!(
            "\n【装配违例】架构沙盘绑定的必选扩展未被真实调用：{}——如结论依赖这些能力，请补调后重派。",
            pending_grants.iter().map(grant_label).collect::<Vec<_>>().join("、"),
        ));
    }
    let _ = kernel.finish_subagent(&subagent_id, status, summary.clone(), error.clone(), content.clone(), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens).await;
    if status == SubagentStatus::Cancelled {
        return Err("子智能体已取消".into());
    }
    if status == SubagentStatus::Failed && content.trim().is_empty() {
        return Err(error.unwrap_or_else(|| "子智能体失败".into()));
    }
    Ok(summary)
}

pub(crate) async fn run(
    kernel: Arc<Kernel>,
    run: RunView,
    config: ChatConfig,
    mut cancel: watch::Receiver<bool>,
) {
    let result: AppResult<bool> = async {
        if kernel.is_cancelled(run.id).await {
            return Ok(false);
        }
        let detail = kernel.session_detail(run.session_id).await?;
        let reports = kernel.recent_subagent_reports(run.session_id).await.unwrap_or_default();
        let run_root = kernel.run_workspace_root(run.id).await;
        // 架构契约在拼装会话时注入：必选 Skill 正文预读 + 硬性调用要求。
        let architecture_brief = kernel.architecture_brief(run_root.as_deref()).await;
        let mut messages = conversation_with_reports(&detail, &run, &reports, config.context_window, &architecture_brief);
        let mut saved_upto = messages.len();
        let mut next_seq = 0i64;
        let tools = kernel.agent_tools(run.mode, run_root.as_deref()).await;
        let provider = kernel.llm_provider(&config);
        let max_rounds = max_tool_rounds();
        let mut last_input_tokens: Option<u64> = None;
        let mut compaction_failures = 0u32;
        // 事后验收追问上限：模型无视欠账连续收尾时避免死循环，2 次后放行。
        // 会话级欠账仍在——下一轮首个实质调用照样被 Layer-B 拦。
        let mut post_nudges = 0u32;
        const POST_NUDGE_LIMIT: u32 = 2;
        for _round in 0..max_rounds {
            if kernel.is_cancelled(run.id).await {
                return Ok(false);
            }
            microcompact(&mut messages);
            if needs_compaction(&messages, config.context_window, last_input_tokens, config.max_output) {
                messages = compact_messages(
                    &kernel,
                    &provider,
                    &config,
                    &messages,
                    &mut CompactionCtx {
                        run: &run,
                        trigger: "auto",
                        cancel: &mut cancel,
                        failures: &mut compaction_failures,
                    },
                )
                .await?;
                saved_upto = messages.len();
            }
            // Watch the cancel signal outside the provider: "stop" must work
            // even if a provider ignores the receiver while awaiting bytes.
            let mut turn_cancel = cancel.clone();
            let outcome = tokio::select! {
                biased;
                _ = turn_cancel.changed() => return Ok(false),
                outcome = stream_turn(
                    &kernel,
                    &provider,
                    &config,
                    &messages,
                    &tools,
                    &mut cancel,
                    run.id,
                ) => outcome,
            };
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) if is_context_error(&error) => {
                    messages = compact_messages(
                        &kernel,
                        &provider,
                        &config,
                        &messages,
                        &mut CompactionCtx {
                            run: &run,
                            trigger: "auto",
                            cancel: &mut cancel,
                            failures: &mut compaction_failures,
                        },
                    )
                    .await?;
                    saved_upto = messages.len();
                    stream_turn(&kernel, &provider, &config, &messages, &tools, &mut cancel, run.id).await?
                }
                Err(error) => return Err(error),
            };
            if !outcome.done {
                return Ok(false);
            }
            if outcome.input_tokens.is_some() {
                last_input_tokens = outcome.input_tokens;
            }
            if outcome.tool_calls.is_empty() {
                // 事后验收欠账：模型想结束时若仍有验收义务未履行，
                // 注入合成追问强制续跑（封顶 POST_NUDGE_LIMIT 次）。
                let outstanding = kernel
                    .architecture_outstanding(run.id, run.session_id, run_root.as_deref(), &run.prompt)
                    .await;
                if !outstanding.is_empty() && post_nudges < POST_NUDGE_LIMIT {
                    post_nudges += 1;
                    let lines = outstanding
                        .iter()
                        .map(|item| format!("- {}", item.message))
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push(ChatMessage::text(
                        "user",
                        format!("【架构规则】本轮尚有未完成的验收义务：\n{lines}\n请先完成上述验收调用，再给出最终答复。"),
                    ));
                    continue;
                }
                // 长度截断的回复与正常完成不可区分是误导性的——
                // finish_reason 透出后向用户标记（DSH 同款语义）。
                if let Some(reason) = outcome.finish_reason.as_deref() {
                    if matches!(reason, "length" | "max_tokens" | "model_context_window_exceeded") {
                        kernel.token(run.id, "\n\n[输出达到生成长度上限（max_tokens），回复已被截断]".into()).await;
                    }
                }
                return Ok(true);
            }
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: outcome.content.clone(),
                tool_calls: Some(outcome.tool_calls.clone()),
                tool_call_id: None,
                name: None,
                images: None,
                thinking: outcome.thinking.clone(),
                is_error: false,
            });
            // Authorization, audit and tool_call events all happen inside the
            // kernel; suspended calls share ONE approval for the whole turn.
            let mut results: Vec<Option<AppResult<String>>> = vec![None; outcome.tool_calls.len()];
            let mut normal_calls: Vec<(String, String)> = Vec::new();
            let mut normal_indices: Vec<usize> = Vec::new();
            let mut spawns = Vec::new();
            let mut asks = Vec::new();
            for (index, call) in outcome.tool_calls.iter().enumerate() {
                if call.name == "spawn_subagent" {
                    if spawns.len() >= max_parallel_subagents() {
                        results[index] = Some(Err(format!("同一轮最多运行 {} 个子智能体", max_parallel_subagents())));
                        continue;
                    }
                    let description = serde_json::from_str::<serde_json::Value>(&call.arguments).ok()
                        .and_then(|value| value.get("description").and_then(|item| item.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "研究子任务".into());
                    let subagent_id = next_subagent_id(run.id);
                    kernel.emit_spawn(run.id, &subagent_id, ToolPhase::Started, &description).await;
                    kernel.record_spawn(run.id, &subagent_id, &description).await;
                    spawns.push((index, subagent_id, call.arguments.clone(), description));
                } else if call.name == "ask_user" {
                    asks.push((index, call.arguments.clone()));
                } else {
                    normal_calls.push((call.name.clone(), call.arguments.clone()));
                    normal_indices.push(index);
                }
            }
            for (position, (index, arguments)) in asks.into_iter().enumerate() {
                let result = if position == 0 {
                    kernel.ask_user(run.id, &arguments, cancel.clone()).await
                } else {
                    Err("同一轮最多提出 1 个问题".into())
                };
                results[index] = Some(result);
            }
            let spawned = join_all(spawns.iter().map(|(_, subagent_id, arguments, _)| {
                run_subagent(kernel.clone(), &config, run.session_id, run.id, subagent_id.clone(), arguments, cancel.clone())
            })).await;
            for ((index, subagent_id, _, description), result) in spawns.into_iter().zip(spawned) {
                let detail = match &result { Ok(text) => text.chars().take(512).collect::<String>(), Err(error) => error.clone() };
                kernel.emit_spawn(run.id, &subagent_id, if result.is_ok() { ToolPhase::Finished } else { ToolPhase::Failed }, &format!("{description} · {detail}")).await;
                results[index] = Some(result);
            }
            if !normal_calls.is_empty() {
                let normal_results = kernel.execute_tool_calls(run.id, &normal_calls).await;
                for (index, result) in normal_indices.into_iter().zip(normal_results) {
                    results[index] = Some(result);
                }
            }
            for (call, result) in outcome.tool_calls.iter().zip(results) {
                let (text, is_error) = match result.unwrap_or_else(|| Err("missing tool result".into())) {
                    Ok(text) => (text, false),
                    Err(error) => (format!("工具调用失败：{error}"), true),
                };
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: bounded_result_text(&text),
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.name.clone()),
                    tool_calls: None,
                    images: None,
                    thinking: None,
                    is_error,
                });
            }
            let new_messages: Vec<RunMessage> = messages[saved_upto..]
                .iter()
                .enumerate()
                .map(|(offset, message)| RunMessage {
                    run_id: run.id,
                    session_id: run.session_id,
                    seq: next_seq + offset as i64,
                    role: message.role.clone(),
                    content: message.content.clone(),
                    tool_calls: message.tool_calls.as_ref().and_then(|calls| serde_json::to_value(calls).ok()),
                    tool_call_id: message.tool_call_id.clone(),
                    name: message.name.clone(),
                    thinking: message.thinking.clone(),
                    is_error: message.is_error,
                })
                .collect();
            if !new_messages.is_empty() {
                next_seq += new_messages.len() as i64;
                saved_upto = messages.len();
                let _ = kernel.save_run_messages(new_messages).await;
            }
        }
        // Tool budget exhausted — give the model one final no-tools turn to
        // summarize what it already gathered instead of failing the run.
        messages.push(ChatMessage::text(
            "user",
            format!("本轮工具调用达到上限（{max_rounds} 轮），任务可能未完成。请基于已收集的信息总结，并提示用户发送「继续」开始新一轮；本轮不要再调用工具。"),
        ));
        let mut final_cancel = cancel.clone();
        let final_turn = tokio::select! {
            biased;
            _ = final_cancel.changed() => return Ok(false),
            outcome = stream_turn(
                &kernel,
                &provider,
                &config,
                &messages,
                &[],
                &mut cancel,
                run.id,
            ) => outcome?,
        };
        Ok(final_turn.done)
    }
    .await;
    let (status, error) = match result {
        Ok(true) => (RunStatus::Completed, None),
        Ok(false) => (RunStatus::Cancelled, None),
        Err(_) if *cancel.borrow() => (RunStatus::Cancelled, None),
        Err(error) => (RunStatus::Failed, Some(config.redact(&error))),
    };
    // All terminal events follow database completion; transport never marks the UI done early.
    kernel.finish(run.id, status, error).await;
}


fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            let cjk = message.content.chars().filter(|character| ('\u{3400}'..='\u{9fff}').contains(character)).count();
            let text = message.content.chars().count();
            let base = cjk + text.saturating_sub(cjk) / 4;
            let tools = message
                .tool_calls
                .as_ref()
                .and_then(|calls| serde_json::to_string(calls).ok())
                .map(|json| json.len() / 4)
                .unwrap_or(0);
            base + tools
        })
        .sum()
}

fn needs_compaction(
    messages: &[ChatMessage],
    context_window: Option<u64>,
    last_input_tokens: Option<u64>,
    max_output: Option<u64>,
) -> bool {
    let Some(window) = context_window.filter(|window| *window > 0) else {
        return false;
    };
    // Provider usage is authoritative: it counts the system prompt, tool
    // schemas and wire framing the char estimate cannot see. Reserve the
    // output budget so the next turn is not pushed over the edge (ZCode
    // provider_usage + output-reserve semantics).
    if let Some(input) = last_input_tokens {
        let reserve = max_output.unwrap_or(4096);
        return input.saturating_add(reserve) as f64 >= window as f64 * 0.9;
    }
    estimate_tokens(messages) as f64 >= window as f64 * 0.8
}

/// Microcompact (ZCode): cheap pass that clips stale tool results before a
/// paid model-summary compaction is even considered. Only results older than
/// the recent tail are touched — fresh output stays verbatim for the model.
const MICRO_KEEP_RECENT: usize = 6;
const MICRO_LIMIT_CHARS: usize = 2000;

fn microcompact(messages: &mut [ChatMessage]) {
    if messages.len() <= MICRO_KEEP_RECENT + 2 {
        return;
    }
    let cutoff = messages.len() - MICRO_KEEP_RECENT;
    for message in messages.iter_mut().take(cutoff) {
        if message.role != "tool" {
            continue;
        }
        let total = message.content.chars().count();
        if total <= MICRO_LIMIT_CHARS {
            continue;
        }
        let head: String = message.content.chars().take(1500).collect();
        let tail: String = message
            .content
            .chars()
            .rev()
            .take(300)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        message.content = format!("{head}\n\n…[工具输出已截断，原长 {total} 字符]…\n\n{tail}");
    }
}

fn is_context_error(error: &str) -> bool {
    let value = error.to_ascii_lowercase();
    value.contains("context length")
        || value.contains("maximum context")
        || value.contains("too many tokens")
        || value.contains("context window")
        || value.contains("context_length_exceeded")
}

fn heuristic_summary(messages: &[ChatMessage]) -> String {
    let mut summary = String::from("[启发式压缩] 早期对话摘要：\n");
    for message in messages {
        summary.push_str(&format!(
            "- {}: {}\n",
            message.role,
            message.content.chars().take(300).collect::<String>()
        ));
    }
    summary.chars().take(12000).collect()
}

async fn summarize_with_model(
    provider: &Arc<dyn LlmProvider>,
    config: &ChatConfig,
    transcript: &str,
    cancel: &mut watch::Receiver<bool>,
) -> Option<String> {
    let summary_config = ChatConfig {
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        provider: config.provider,
        reasoning: None,
        max_output: Some(1024),
        context_window: None,
        session: config.session.clone(),
    };
    let messages = vec![
        ChatMessage::text(
            "system",
            "你是上下文压缩器。把下面的历史压缩成简洁中文摘要，必须保留：已完成的工作、涉及的文件路径、未完成事项、关键结论与风险。不要调用工具，不要编造。",
        ),
        ChatMessage::text("user", transcript),
    ];
    let (tx, mut rx) = mpsc::unbounded_channel();
    let streamed = tokio::time::timeout(
        Duration::from_secs(60),
        provider.clone().stream(&summary_config, &messages, &[], cancel, tx),
    )
    .await
    .ok()?
    .ok()?;
    if !streamed.done {
        return None;
    }
    let mut text = String::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::Token(token) = event {
            text.push_str(&token);
        }
    }
    (!text.trim().is_empty()).then_some(text)
}

/// Compaction call context: the owning run, the audit trigger label, the
/// cancel channel and the consecutive-failure breaker counter.
struct CompactionCtx<'a> {
    run: &'a RunView,
    trigger: &'a str,
    cancel: &'a mut watch::Receiver<bool>,
    failures: &'a mut u32,
}

async fn compact_messages(
    kernel: &Arc<Kernel>,
    provider: &Arc<dyn LlmProvider>,
    config: &ChatConfig,
    messages: &[ChatMessage],
    ctx: &mut CompactionCtx<'_>,
) -> AppResult<Vec<ChatMessage>> {
    let keep = 8.min(messages.len().saturating_sub(1));
    let mut split = messages.len().saturating_sub(keep);
    // 切点不能落在 tool 消息上：tool_result 与其 assistant tool_calls 必须
    // 同侧，否则保留段以孤儿 tool 消息开头，两个 provider 都会 400。
    // 前移会破坏配对，因此只能后移（把整段调用+结果一起交给摘要）。
    while split < messages.len() && messages[split].role == "tool" {
        split += 1;
    }
    if split <= 1 || split >= messages.len() {
        return Ok(messages.to_vec());
    }
    let pre = estimate_tokens(messages) as u64;
    let older = &messages[1..split];
    let transcript = older
        .iter()
        .map(|message| format!("{}: {}", message.role, message.content.chars().take(1000).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");
    let operation_id = format!("compact-{}-{}", ctx.run.id, crate::domain::unix_now());
    let running = ContextCompaction {
        operation_id: operation_id.clone(),
        session_id: ctx.run.session_id,
        through_run_id: ctx.run.id - 1,
        summary: String::new(),
        trigger: ctx.trigger.into(),
        status: "running".into(),
        pre_tokens: pre,
        post_tokens: None,
        created_at: crate::domain::unix_now(),
    };
    let _ = kernel.save_compaction(running.clone()).await;
    // 熔断（ZCode）：付费 summarize 连续失败 3 次后不再尝试，直接走启发式
    // 摘要——否则每一轮都会为一个注定失败的请求付一次全量输入费用。
    const COMPACTION_FAILURE_BREAKER: u32 = 3;
    let summary = if *ctx.failures >= COMPACTION_FAILURE_BREAKER {
        heuristic_summary(older)
    } else {
        match summarize_with_model(provider, config, &transcript, ctx.cancel).await {
            Some(text) => {
                *ctx.failures = 0;
                text
            }
            None => {
                *ctx.failures += 1;
                heuristic_summary(older)
            }
        }
    };
    let mut next = vec![
        messages[0].clone(),
        ChatMessage::text("system", format!("[上下文压缩摘要]\n{summary}")),
    ];
    next.extend_from_slice(&messages[split..]);
    let post = estimate_tokens(&next) as u64;
    let _ = kernel
        .save_compaction(ContextCompaction {
            operation_id,
            summary,
            status: "completed".into(),
            post_tokens: Some(post),
            ..running
        })
        .await;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_surface_prices_tool_call_argument_text_like_content() {
        // Tool-call 参数是 provider 计费的输出 token：参数流期间
        // liveSurfaceTokens 必须保持非零，≈tok/s 才不会停摆。
        let mut cjk = 0;
        let mut other = 0;
        count_surface("{\"path\":\"src/a.ts\",\"content\":\"console.log(1)\"}", &mut cjk, &mut other);
        assert_eq!(cjk, 0);
        assert!(other > 0);
        let est = cjk + other.div_ceil(4);
        assert!(est > 0);
    }

    #[test]
    fn provider_usage_gates_compaction_with_an_output_reserve() {
        let messages = vec![ChatMessage::text("user", "hi")];
        let window = Some(200_000);
        // input + max_output(4k) < 90% window → no compaction.
        assert!(!needs_compaction(&messages, window, Some(100_000), Some(4_096)));
        // 180k input + 4k reserve ≥ 90% of 200k → compact.
        assert!(needs_compaction(&messages, window, Some(180_000), Some(4_096)));
        // Larger output reserves trigger earlier.
        assert!(needs_compaction(&messages, window, Some(160_000), Some(32_000)));
        // No provider sample → fall back to the char estimate.
        assert!(!needs_compaction(&messages, window, None, Some(4_096)));
        assert!(!needs_compaction(&messages, None, Some(190_000), Some(4_096)));
    }

    #[test]
    fn microcompact_clips_only_stale_tool_results() {
        let big = "x".repeat(5000);
        let mut messages = vec![ChatMessage::text("system", "sys")];
        for index in 0..8 {
            messages.push(ChatMessage {
                role: "tool".into(),
                content: big.clone(),
                tool_call_id: Some(format!("call-{index}")),
                name: Some("read_file".into()),
                tool_calls: None,
                images: None,
                thinking: None,
                is_error: false,
            });
        }
        microcompact(&mut messages);
        // Older tool results (beyond the recent tail) are clipped…
        assert!(messages[1].content.contains("工具输出已截断"), "{}", messages[1].content.len());
        // …while the six most recent stay verbatim.
        assert_eq!(messages[8].content.len(), 5000);
        assert_eq!(messages[7].content.len(), 5000);
    }

    #[test]
    fn count_surface_keeps_cjk_density_for_argument_text() {
        let mut cjk = 0;
        let mut other = 0;
        count_surface("{\"path\":\"目录/文件.txt\"}", &mut cjk, &mut other);
        assert_eq!(cjk, 4);
    }

    fn fixture(status: RunStatus, output: &str) -> (SessionDetail, RunView) {
        let prior = RunView {
            id: 1,
            session_id: 7,
            prompt: "上一轮任务".into(),
            mode: Mode::Code,
            status,
            output: output.into(),
            error: None,
            created_at: 1,
        };
        let current = RunView {
            id: 2,
            session_id: 7,
            prompt: "继续".into(),
            mode: Mode::Code,
            status: RunStatus::Running,
            output: String::new(),
            error: None,
            created_at: 2,
        };
        let detail = SessionDetail {
            reasoning: Default::default(),
            parts: Default::default(),
            session: crate::domain::SessionView {
                id: 7,
                title: "session".into(),
                updated_at: 2,
                run_count: 2,
                project_id: 1,
                archived: false,
            },
            runs: vec![prior],
            plans: Vec::new(),
            metrics: Vec::new(),
            compactions: Vec::new(),
            run_messages: Vec::new(),
            tools: vec![ToolCallView {
                run_id: 1,
                tool: "write_file".into(),
                detail: "src/a.ts".into(),
                allowed: true,
                subagent_id: None,
                pending: None,
                target: None,
            }],
        };
        (detail, current)
    }

    #[test]
    fn stored_run_messages_are_replayed_in_order() {
        let (mut detail, current) = fixture(RunStatus::Completed, "final answer");
        detail.run_messages.push(RunMessage {
            run_id: 1,
            session_id: 7,
            seq: 0,
            role: "assistant".into(),
            content: String::new(),
            tool_calls: Some(serde_json::json!([
                { "id": "call-1", "name": "read_file", "arguments": "{\"path\":\"a.ts\"}" }
            ])),
            tool_call_id: None,
            name: None,
            thinking: None,
            is_error: false,
        });
        detail.run_messages.push(RunMessage {
            run_id: 1,
            session_id: 7,
            seq: 1,
            role: "tool".into(),
            content: "file body".into(),
            tool_calls: None,
            tool_call_id: Some("call-1".into()),
            name: Some("read_file".into()),
            thinking: None,
            is_error: false,
        });
        let messages = conversation(&detail, &current, None);
        assert!(messages.iter().any(|message| message.role == "tool" && message.content == "file body"));
        assert!(!messages.iter().any(|message| message.content.contains("上一轮实际执行过的工具")));
    }

    #[test]
    fn completed_compaction_replaces_the_runs_it_covers() {
        let (mut detail, current) = fixture(RunStatus::Completed, "done");
        detail.compactions.push(ContextCompaction {
            operation_id: "compact-1".into(),
            session_id: 7,
            through_run_id: 1,
            summary: "早前完成了 A".into(),
            trigger: "auto".into(),
            status: "completed".into(),
            pre_tokens: 100,
            post_tokens: Some(20),
            created_at: 1,
        });
        let messages = conversation(&detail, &current, None);
        assert!(messages.iter().any(|message| message.role == "system"
            && message.content.contains("早前完成了 A")));
        assert!(!messages.iter().any(|message| message.content.contains("上一轮任务")));
    }

    #[test]
    fn interrupted_runs_are_replayed_with_partial_output_and_tool_summary() {
        let (detail, current) = fixture(RunStatus::Cancelled, "已经写了 a.ts");
        let messages = conversation(&detail, &current, None);
        assert!(messages.iter().any(|message| message.role == "assistant"
            && message.content.contains("上一轮被用户中断")
            && message.content.contains("已经写了 a.ts")));
        assert!(messages.iter().any(|message| message.role == "system"
            && message.content.contains("write_file")
            && message.content.contains("src/a.ts")));
    }

    #[test]
    fn completed_runs_without_text_still_replay_their_tool_actions() {
        let (detail, current) = fixture(RunStatus::Completed, "");
        let messages = conversation(&detail, &current, None);
        assert!(messages.iter().any(|message| message.role == "assistant"
            && message.content.contains("没有最终文字输出")));
        assert!(messages.iter().any(|message| message.role == "system"
            && message.content.contains("write_file")));
    }

    #[test]
    fn running_prior_runs_are_not_replayed() {
        let (detail, current) = fixture(RunStatus::Running, "partial");
        let messages = conversation(&detail, &current, None);
        assert!(!messages.iter().any(|message| message.content.contains("上一轮任务")));
    }

    #[test]
    fn replay_budget_scales_with_the_context_window() {
        let small = replay_budget_bytes(Some(128_000));
        let large = replay_budget_bytes(Some(1_000_000));
        assert!(large > small * 5, "{large} vs {small}");
        assert_eq!(replay_budget_bytes(None), small);
        assert_eq!(replay_budget_bytes(Some(0)), small);
        assert_eq!(replay_budget_bytes(Some(u64::MAX)), 8 * 1024 * 1024);
        assert!(small >= 65_536);
    }

    #[test]
    fn child_sessions_are_distinct_from_the_parent() {
        assert_eq!(child_session_id(Some("maestro-7"), "sa-7-1"), "maestro-7-sub-sa-7-1");
        assert_ne!(
            child_session_id(Some("maestro-7"), "sa-7-1"),
            child_session_id(Some("maestro-7"), "sa-7-2")
        );
        assert_eq!(child_session_id(None, "sa-7-1"), "maestro-sub-sa-7-1");
    }

    #[test]
    fn subagent_tool_budget_defaults_and_clamps() {
        let mut definition = crate::domain::AgentDefinition::builtin("explore").unwrap();
        assert_eq!(subagent_tool_budget(&definition), crate::domain::DEFAULT_SUBAGENT_TOOL_BUDGET as usize);
        definition.tool_budget = Some(0);
        assert_eq!(subagent_tool_budget(&definition), 1);
        definition.tool_budget = Some(9999);
        assert_eq!(subagent_tool_budget(&definition), crate::domain::MAX_SUBAGENT_TOOL_BUDGET as usize);
        definition.tool_budget = Some(60);
        assert_eq!(subagent_tool_budget(&definition), 60);
    }
}
