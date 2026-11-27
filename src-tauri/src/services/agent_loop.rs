//! Agent loop (ReAct): think → call tools → observe → answer. The LLM
//! adapter streams normalized `StreamEvent`s over a channel; this service
//! relays them to the kernel as typed domain events, executes tool calls via
//! the kernel's authorized path, and feeds results back for the next turn.
/// 工具执行产出的模型可见载荷（与 kernel::ToolPayload 同构——跨模块
/// 复刻别名而非导出，保持 kernel 内部类型私密）。
type ToolPayload = (String, Vec<String>);

use std::{collections::HashMap, sync::{atomic::{AtomicU64, AtomicUsize, Ordering}, Arc}, time::{Duration, Instant}};

use futures_util::future::join_all;

use tokio::sync::{mpsc, watch};

use crate::{
    adapters::llm::{ChatConfig, LlmProvider, StreamEvent, TurnOutcome},
    domain::{
        AppResult, ChatMessage, ContextCompaction, ExtensionKind, Mode, RunMessage, RunMetrics,
        RunStatus, RunView, SessionDetail, SubagentStatus, SubagentTokenKind, SubagentView,
        ToolCallSpec, ToolCallView, ToolPhase, ToolSpec,
    },
    services::{kernel::Kernel, tool_router::{ResolvedSquad, SubagentGrant}},
};

/// Safety cap for one run; can be raised with MAESTRO_MAX_TOOL_ROUNDS.
fn max_tool_rounds() -> usize {
    std::env::var("MAESTRO_MAX_TOOL_ROUNDS").ok().and_then(|value| value.parse::<usize>().ok()).filter(|value| *value >= 8).unwrap_or(64)
}
const MAX_TOOL_RESULT_BYTES: usize = 65536;

/// 停滞检测：同一工具调用（名称 + 参数字节完全一致）连续出现时的渐进处置。
/// 口径与 OpenCode DOOM_LOOP 一致——字节级精确匹配，不做 JSON 规范化；
/// 交错模式（A,B,A,B）不触发。三级渐进：
/// - streak == DOOM_WARN（3）：照常执行，结果尾部追加「请说明原因/改变策略」
/// - streak 4–5：拦截不执行，合成 Err 结果喂回模型（error-as-observation）
/// - streak >= DOOM_ABORT（6）：硬终止本轮 run（可见标记 + 审计）
const DOOM_WARN: u32 = 3;
const DOOM_BLOCK: u32 = 4;
const DOOM_ABORT: u32 = 6;

#[derive(Default)]
struct DoomGuard {
    signature: String,
    streak: u32,
}

impl DoomGuard {
    /// 记录一次调用并返回其当前连续计数；签名不同则重置。
    fn observe(&mut self, call: &ToolCallSpec) -> u32 {
        let signature = format!("{}\u{1f}{}", call.name, call.arguments);
        if self.signature == signature {
            self.streak += 1;
        } else {
            self.signature = signature;
            self.streak = 1;
        }
        self.streak
    }
}

fn doom_warn_note(streak: u32) -> String {
    format!("\n\n【系统提示】检测到该调用已连续 {streak} 次完全相同。若是有意重试请在后续说明原因；否则请改变策略。")
}

fn doom_block_error(name: &str, streak: u32) -> String {
    format!("该调用与此前连续 {streak} 次完全相同，已被停滞检测拦截（未执行 {name}）。请改变策略——换参数、换工具，或直接说明卡点。")
}

fn doom_abort_marker(name: &str, streak: u32) -> String {
    format!("\n\n[检测到工具调用停滞：{name} 已连续 {streak} 次完全相同，本轮已终止。请检查策略后发送「继续」]")
}

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
/// 如实告知双约束——模型无法管理它看不见的限制；一轮可并行多调用是
/// 小模型调研效率的关键，必须显式说明。
fn subagent_budget_hint(steps: u32, tool_budget: usize) -> String {
    format!("效率约束：你最多有 {steps} 轮对话和 {tool_budget} 次工具调用。一轮可以并行调用多个工具——先 repo_map 拿仓库全貌，再每轮批量并行读取多个关键文件；不要逐文件顺序读取，不要重复读取同一文件。接近上限时立即输出已有结论；最后一轮工具将被禁用，必须输出纯文本的最终结论、证据路径与风险。")
}

/// A child that stops producing progress must not hold the parent turn forever.
/// The default lives in `domain::DEFAULT_SUBAGENT_TOOL_BUDGET`; definitions may
/// override it via `tool_budget`.
fn subagent_tool_budget(definition: &crate::domain::AgentDefinition, defaults: &crate::domain::SubagentDefaults) -> usize {
    definition.tool_budget.or(defaults.tool_budget).unwrap_or(crate::domain::DEFAULT_SUBAGENT_TOOL_BUDGET).clamp(1, crate::domain::MAX_SUBAGENT_TOOL_BUDGET) as usize
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
        Mode::Plan => "你处于计划模式。只读工具直接执行；白名单只读命令（git/rg/cat/ls 等）可经 run_command 调用，但每次调用都会挂起等待用户逐项批准，批准后才会真实执行。写文件、补丁、非白名单命令与非只读扩展工具不可用，也不得声称已应用变更或已执行。",
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
    // 图片回放预算：仅最近 2 个含图用户轮保留 images（每轮最多 4 张）——
    // 单图约 1-2k token，全量回放会挤占文本预算；更早的含图轮降级为文字提示，
    // 模型由此知道"曾经有图"而不是静默丢失。（included[0] 为最新一轮。）
    let mut image_replay: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for (item, _, _, _) in included.iter() {
        if detail.run_images.get(&item.id).is_some_and(|images| !images.is_empty()) {
            image_replay.insert(item.id);
            if image_replay.len() >= 2 {
                break;
            }
        }
    }
    for (item, output, summary, run_messages) in included.into_iter().rev() {
        let mut prompt_text = item.prompt.clone();
        let mut user_images: Option<Vec<String>> = None;
        if let Some(images) = detail.run_images.get(&item.id).filter(|images| !images.is_empty()) {
            if image_replay.contains(&item.id) {
                user_images = Some(images.iter().take(4).cloned().collect());
            } else {
                prompt_text.push_str("\n[本轮曾附图片，因上下文预算未回放]");
            }
        }
        let mut user_message = ChatMessage::text("user", prompt_text);
        user_message.images = user_images;
        messages.push(user_message);
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
    // 当前轮图片全量携带（validate 已限 ≤8 张）——这是用户刚刚粘贴/选择的
    // 视觉上下文，正是本轮工作的输入。
    let mut current = ChatMessage::text("user", run.prompt.clone());
    current.images = detail
        .run_images
        .get(&run.id)
        .filter(|images| !images.is_empty())
        .cloned();
    messages.push(current);
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

/// 每轮并行上限：~/.maestro/agents.json > env MAESTRO_MAX_SUBAGENTS > 2（范围 1-4）。
fn resolve_max_parallel(defaults: &crate::domain::SubagentDefaults) -> usize {
    if let Some(value) = defaults.max_parallel {
        return (value as usize).clamp(1, 4);
    }
    std::env::var("MAESTRO_MAX_SUBAGENTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=4).contains(value))
        .unwrap_or(2)
}

/// Run 级子智能体总额度（含军团成员/孙级），防扇出：
/// agents.json > env MAESTRO_MAX_TOTAL_SUBAGENTS > 8（范围 1-16）。
fn resolve_max_total(defaults: &crate::domain::SubagentDefaults) -> usize {
    if let Some(value) = defaults.max_total {
        return (value as usize).clamp(1, 16);
    }
    std::env::var("MAESTRO_MAX_TOTAL_SUBAGENTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=16).contains(value))
        .unwrap_or(8)
}

/// steps 解析链：definition → 全局默认 → 8（0 视为未设置）。
fn resolve_steps(definition: &crate::domain::AgentDefinition, defaults: &crate::domain::SubagentDefaults) -> u32 {
    definition.steps.or(defaults.steps).filter(|value| *value > 0).unwrap_or(8)
}

/// 预算原子扣减：到 0 返回 false（fetch_update 失败时不写回，语义精确）。
fn try_spawn_budget(budget: &AtomicUsize) -> bool {
    budget
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_sub(1))
        .is_ok()
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

/// 指挥系统提示追加：军团编排契约——成员 enum、必选派发、只读语义。
fn squad_dispatch_prompt(squad: &ResolvedSquad, defaults: &crate::domain::SubagentDefaults) -> String {
    let required: Vec<&str> = squad
        .members
        .iter()
        .filter(|member| member.required)
        .map(|member| member.agent.as_str())
        .collect();
    let optional: Vec<&str> = squad
        .members
        .iter()
        .filter(|member| !member.required)
        .map(|member| member.agent.as_str())
        .collect();
    format!(
        "\n【军团编排】你是本军团的指挥：可用 spawn_subagent 向成员派发只读研究任务（同一轮最多 {} 个，本 run 总名额有限）。必选成员（收尾前必须实际派发）：{}。可选成员：{}。成员只读并返回摘要作为证据；不要试图派发成员之外的类型。",
        resolve_max_parallel(defaults),
        if required.is_empty() { "无".into() } else { required.join("、") },
        if optional.is_empty() { "无".into() } else { optional.join("、") },
    )
}

fn pending_members_reminder(pending: &[String]) -> String {
    format!(
        "【军团编排】必选成员尚未派发：{}。请继续调用 spawn_subagent（subagent_type=成员名）完成派发后才能收尾。",
        pending.join("、")
    )
}

/// 欠账提醒合并：装配必选 + 必派成员——子循环两处提醒入口共用。
fn pending_reminder(grants: &[SubagentGrant], members: &[String]) -> String {
    let mut lines = Vec::new();
    if !members.is_empty() {
        lines.push(pending_members_reminder(members));
    }
    if !grants.is_empty() {
        lines.push(pending_grants_reminder(grants));
    }
    lines.join("\n")
}

/// 并行军团：无指挥——一次 spawn 调用 fan-out 全部成员，结果合并为一条。
/// 每个成员照常走 run_subagent（预算、装配、取消全继承）。
#[allow(clippy::too_many_arguments)]
async fn run_squad_parallel(
    kernel: Arc<Kernel>,
    config: &ChatConfig,
    session_id: i64,
    run_id: i64,
    squad: &ResolvedSquad,
    prompt: &str,
    description: &str,
    parent_cancel: watch::Receiver<bool>,
    spawn_budget: Arc<AtomicUsize>,
) -> AppResult<String> {
    if squad.members.is_empty() {
        return Err("军团没有成员子智能体".into());
    }
    // 两段式：先把每个成员的派发参数/授权存进 jobs，再统一建 future（借用 jobs 内部数据）。
    let mut jobs: Vec<(String, String, String, String, Vec<SubagentGrant>)> = Vec::new();
    for member in &squad.members {
        let member_id = next_subagent_id(run_id);
        let member_desc = format!("{description}·{}", member.agent);
        kernel.emit_spawn(run_id, &member_id, ToolPhase::Started, &member_desc).await;
        let mut grants = member.grants.clone();
        grants.extend(squad.squad_grants.clone());
        let arguments = serde_json::json!({
            "subagent_type": member.agent,
            "description": member_desc,
            "prompt": prompt,
        })
        .to_string();
        kernel
            .record_spawn(
                run_id,
                &member_id,
                &member_desc,
                &crate::services::tool_router::ToolRouter::history_target("spawn_subagent", &arguments),
            )
            .await;
        jobs.push((member_id, member.agent.clone(), member_desc, arguments, grants));
    }
    let futures: Vec<_> = jobs
        .iter()
        .map(|(member_id, _, _, arguments, grants)| {
            Box::pin(run_subagent(
                kernel.clone(),
                config,
                session_id,
                run_id,
                member_id.clone(),
                arguments,
                parent_cancel.clone(),
                spawn_budget.clone(),
                false,
                grants.clone(),
            ))
        })
        .collect();
    let results = join_all(futures).await;
    let mut merged = String::new();
    for ((member_id, agent, member_desc, _, _), result) in jobs.into_iter().zip(results) {
        kernel
            .emit_spawn(
                run_id,
                &member_id,
                if result.is_ok() { ToolPhase::Finished } else { ToolPhase::Failed },
                &member_desc,
            )
            .await;
        merged.push_str(&format!(
            "【{agent}】\n{}\n\n",
            result.unwrap_or_else(|error| error)
        ));
    }
    Ok(merged.trim_end().to_string())
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
pub(crate) async fn run_subagent(
    kernel: Arc<Kernel>,
    config: &ChatConfig,
    session_id: i64,
    run_id: i64,
    subagent_id: String,
    arguments_json: &str,
    mut parent_cancel: watch::Receiver<bool>,
    // 全 run 子智能体总额度（含成员/孙级）——每个真实子循环入口各扣 1。
    spawn_budget: Arc<AtomicUsize>,
    // 成员/孙级递归传 false：军团成员只引用 agent 定义，禁止军团套军团。
    resolve_squad: bool,
    // 军团级/成员级图谱补充授权（并入本调用 grants）。
    extra_grants: Vec<SubagentGrant>,
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
    let run_root = kernel.run_workspace_root(run_id).await;
    // 解析顺序：agent 定义优先，未命中再查军团（同名时 agent 胜出）。
    let agent_def = available
        .iter()
        .find(|agent| agent.name == subagent_type)
        .cloned()
        .or_else(|| crate::domain::AgentDefinition::builtin(&subagent_type));
    let squad = if resolve_squad && agent_def.is_none() {
        kernel.squad_runtime(run_root.as_deref(), &subagent_type).await
    } else {
        None
    };
    if let Some(squad) = &squad {
        if squad.coordinator.is_none() {
            // 并行军团：无指挥——本调用 fan-out 全部成员，结果合并为一条。
            return run_squad_parallel(
                kernel, config, session_id, run_id, squad, &prompt, &description,
                parent_cancel, spawn_budget,
            )
            .await;
        }
    }
    let (definition, squad) = match (agent_def, squad) {
        (Some(definition), _) => (definition, None),
        (None, Some(squad)) => {
            let coordinator = squad.coordinator.clone().unwrap_or_default();
            let definition = available
                .iter()
                .find(|agent| agent.name == coordinator)
                .cloned()
                .or_else(|| crate::domain::AgentDefinition::builtin(&coordinator))
                .ok_or_else(|| format!("军团 {subagent_type} 的指挥 {coordinator} 未在子智能体定义中（可用：{names}）"))?;
            (definition, Some(squad))
        }
        (None, None) => {
            return Err(format!("未知子智能体类型：{subagent_type}；可用：{names}"));
        }
    };
    if definition.hidden || definition.mode != "subagent" {
        return Err(format!("子智能体 {subagent_type} 未启用：hidden 或 mode 不是 subagent"));
    }
    // 每个真实子循环入口各扣 1 份 run 总额度（军团成员/孙级同样计）。
    if !try_spawn_budget(&spawn_budget) {
        return Err("本 run 子智能体总额度已用完（可在「设置 → 子智能体」或经 MAESTRO_MAX_TOTAL_SUBAGENTS 调整）".into());
    }
    // 全局默认（~/.maestro/agents.json）：per-agent 留空字段的第二级回退。
    let defaults = kernel.agent_defaults()?;
    let child_config = ChatConfig {
        api_key: config.api_key.clone(), base_url: config.base_url.clone(),
        model: definition.model.clone().or_else(|| defaults.model.clone()).unwrap_or_else(|| config.model.clone()),
        provider: config.provider, reasoning: config.reasoning.clone(),
        max_output: config.max_output,
        context_window: config.context_window,
        session: Some(child_session_id(config.session.as_deref(), &subagent_id)),
        // 继承父 run 视觉门控：子智能体模型覆盖若为非视觉模型，图像可能
        // 抵达不支持的上游（v1 已文档化限制——建议视觉链路模型保持一致）。
        vision_capable: config.vision_capable,
    };
    let provider = kernel.llm_provider(&child_config);
    // 沙盘装配：Extension → Subagent 绑定给该类型子智能体授权扩展能力
    // （Skill 预注入 + mcp__/load_skill 进白名单），required 在子循环内强制真实调用。
    let mut grants = kernel.subagent_grants(run_root.as_deref(), &definition.name).await;
    grants.extend(extra_grants);
    if let Some(squad) = &squad {
        // 军团级授权全员生效（含指挥）；指挥独享军团内 ext→指挥 的装配绑定。
        grants.extend(squad.squad_grants.clone());
        grants.extend(squad.coordinator_grants.clone());
    }
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
    // 只读研究契约：子智能体清单再按 effect 过滤——run_command 开特例：
    // 它的效应是 ProcessExecution，但执行侧 gate_call 已把可执行面收窄到
    // 白名单只读命令（无 shell 直执行），研究与 git 感知离不开它。
    let mut tools: Vec<_> = Vec::new();
    for spec in kernel.agent_tools(Mode::Plan, run_root.as_deref()).await {
        if spec.name == "spawn_subagent" || (!base_open && !tool_allowed(&spec.name)) {
            continue;
        }
        let read_only = matches!(kernel.tool_effect(&spec.name).await, Ok(crate::domain::ToolEffect::ReadOnly));
        if !read_only && spec.name != "run_command" {
            continue;
        }
        tools.push(spec);
    }
    if let Some(squad) = &squad {
        // 指挥的派发工具：enum 收窄为 dispatch 边连出的成员（编排即声明）。
        let member_names: Vec<String> = squad.members.iter().map(|member| member.agent.clone()).collect();
        let required: Vec<&str> = squad.members.iter().filter(|m| m.required).map(|m| m.agent.as_str()).collect();
        let optional: Vec<&str> = squad.members.iter().filter(|m| !m.required).map(|m| m.agent.as_str()).collect();
        tools.push(ToolSpec {
            name: "spawn_subagent".into(),
            description: format!(
                "向军团成员派发只读研究任务并回收摘要（不写文件、不执行命令）。description 是短标题，prompt 是具体任务。必选成员：{}；可选：{}。",
                if required.is_empty() { "无".into() } else { required.join("、") },
                if optional.is_empty() { "无".into() } else { optional.join("、") },
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "subagent_type": { "type": "string", "enum": member_names, "description": "军团成员类型" },
                    "description": { "type": "string" },
                    "prompt": { "type": "string" }
                },
                "required": ["description", "prompt"]
            }),
        });
    }
    let allowed_names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let tool_budget = subagent_tool_budget(&definition, &defaults);
    let steps = resolve_steps(&definition, &defaults);
    let dispatch_prompt = squad.as_ref().map(|squad| squad_dispatch_prompt(squad, &defaults)).unwrap_or_default();
    let system_prompt = format!("{}
{}{}{}", if definition.prompt.trim().is_empty() { DEFAULT_SUBAGENT_PROMPT.to_string() } else { definition.prompt.clone() }, subagent_budget_hint(steps, tool_budget), subagent_assembly_prompt(&grants), dispatch_prompt);
    // 必选装配：子循环内逐轮强制，全部真实调用后才允许收尾。
    let mut pending_grants: Vec<SubagentGrant> = grants
        .into_iter()
        .filter(|grant| grant.binding.required && grant.binding.missing.is_none())
        .collect();
    // 指挥态：必选成员未实际派发前同样不许收尾（与装配核销同套机制）。
    let mut pending_members: Vec<String> = squad
        .as_ref()
        .map(|squad| {
            squad
                .members
                .iter()
                .filter(|member| member.required)
                .map(|member| member.agent.clone())
                .collect()
        })
        .unwrap_or_default();
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
        // 空轮提醒只发一次：网关丢弃工具调用或模型空响应时给一次自愈
        // 机会，再空则带诊断收尾而不是静默返回空报告。
        let mut empty_reminded = false;
        // 停滞检测：与父循环同口径——同名同参连续调用软警告→拦截→
        // 硬终止，防止子智能体在小预算内反复相同调用空转。
        let mut doom = DoomGuard::default();
        let timeout_message = format!("子智能体超时（{}s）", timeout.as_secs());
        // 收尾轮哨兵：最后一轮或预算耗尽时触发一次无工具纯文本轮——
        // 模型必须综合已收集证据输出结论，而不是直接返回旁白碎片。
        // 收尾轮只跑一次（synthesis_done 防重复燃烧 LLM 调用）；超时仍
        // 是硬墙，不假装有时间收尾。
        let mut synthesis_done = false;
        for iteration in 0..steps {
            if *cancel.borrow() {
                return ChildOutcome { status: SubagentStatus::Cancelled, content: notes, error: Some("子智能体已取消".into()), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(timeout_message), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            if synthesis_done {
                break;
            }
            let budget_out = tool_count >= tool_budget;
            let synthesis_round = budget_out || iteration + 1 >= steps;
            if synthesis_round {
                synthesis_done = true;
                messages.push(ChatMessage::text("user", "【系统提示】这是最后一轮：工具已全部禁用。请立即基于已收集的证据输出最终结论——关键发现、证据文件路径与风险；不要尝试调用任何工具。"));
            }
            let round_tools: &[ToolSpec] = if synthesis_round { &[] } else { &tools };
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
                    provider.clone().stream(&child_config, &messages, round_tools, &mut cancel, tx),
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
                            if !notes.is_empty() {
                                notes.push_str("\n\n");
                            }
                            notes.push_str(&partial);
                        }
                        return ChildOutcome { status: SubagentStatus::Failed, content: notes, error: Some(error), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
                    }
                    Err(_) => {
                        if !partial.trim().is_empty() {
                            if !notes.is_empty() {
                                notes.push_str("\n\n");
                            }
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
                if !notes.is_empty() {
                    notes.push_str("\n\n");
                }
                notes.push_str(&text);
                if notes.len() > SUBAGENT_PARTIAL_BYTES {
                    notes = notes.chars().take(SUBAGENT_PARTIAL_BYTES).collect();
                }
            }
            // 收尾轮拦截：工具已剥离，正文即最终综合——无论是否还有必选
            // 装配/必派成员未清，结论都必须送达父级（违例标注在循环外统一
            // 追加）；模型仍空响应时落入既有空轮诊断路径。
            if synthesis_round && !text.trim().is_empty() {
                let error = if budget_out {
                    format!("达到工具预算（{tool_budget} 次），已返回收尾结论")
                } else {
                    format!("达到轮次上限（{steps} 轮），已返回收尾结论")
                };
                return ChildOutcome { status: SubagentStatus::Completed, content: text, error: Some(error), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            if outcome.tool_calls.is_empty() {
                // 收尾轮空转（正文为空——非空已在上方拦截返回）：没有更多轮次
                // 可补救，提醒续跑只会撞上 synthesis_done 直接出循环。留审计后
                // 如实以既有碎片收尾，耗尽原因按真实触发源区分。
                if synthesis_round {
                    kernel
                        .audit_turn_anomaly(
                            run_id,
                            format!(
                                "subagent={} synthesis_empty finish_reason={} dropped_tool_calls={}",
                                subagent_id,
                                outcome.finish_reason.as_deref().unwrap_or("none"),
                                outcome.dropped_tool_calls
                            ),
                        )
                        .await;
                    let error = if budget_out {
                        format!("达到工具预算（{tool_budget} 次），已返回部分结论")
                    } else {
                        format!("达到轮次上限（{steps} 轮），已返回部分结论")
                    };
                    return ChildOutcome { status: SubagentStatus::Completed, content: notes, error: Some(error), input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
                }
                // 必选装配/必派成员未清：不收尾，附提醒续跑（步数上限兜底）——
                // 与父级「正文已给但调用必须真实发生」同一口径。
                if !pending_grants.is_empty() || !pending_members.is_empty() {
                    messages.push(ChatMessage::text("user", pending_reminder(&pending_grants, &pending_members)));
                    continue;
                }
                // 空轮/丢调用异常：完成一轮却零可见正文，或调用被丢弃——
                // 提醒一次仍空则以诊断内容收尾，父级不再拿到空报告。
                if text.trim().is_empty() {
                    if !empty_reminded {
                        empty_reminded = true;
                        kernel
                            .audit_turn_anomaly(
                                run_id,
                                format!(
                                    "subagent={} finish_reason={} content_bytes={} dropped_tool_calls={}",
                                    subagent_id,
                                    outcome.finish_reason.as_deref().unwrap_or("none"),
                                    text.len(),
                                    outcome.dropped_tool_calls
                                ),
                            )
                            .await;
                        messages.push(ChatMessage::text(
                            "user",
                            "【系统提示】上一轮没有产生任何可见回复或可执行的工具调用。请直接以文本给出最终结论。",
                        ));
                        continue;
                    }
                    let content = format!(
                        "子智能体未产生可见输出（finish_reason: {}{}）",
                        outcome.finish_reason.as_deref().unwrap_or("unknown"),
                        if outcome.dropped_tool_calls > 0 {
                            format!("，丢弃 {} 个工具调用碎片", outcome.dropped_tool_calls)
                        } else {
                            String::new()
                        }
                    );
                    return ChildOutcome { status: SubagentStatus::Completed, content, error: None, input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens, last_input_tokens, tool_count };
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
            let mut results: Vec<Option<AppResult<ToolPayload>>> = vec![None; outcome.tool_calls.len()];
            let mut runnable: Vec<(String, String)> = Vec::new();
            let mut runnable_indices: Vec<usize> = Vec::new();
            // 指挥的军团成员派发：与父循环同构——拦截 spawn 后 join_all 递归。
            let mut member_spawns: Vec<(usize, String, crate::services::tool_router::ResolvedSquadMember, String, String)> = Vec::new();
            // 停滞检测 pre-pass：与父循环同一阈值；Block 不执行改喂合成
            // 错误，Abort 在本轮结果写回后带诊断收尾。
            let mut doom_streaks: Vec<u32> = Vec::with_capacity(outcome.tool_calls.len());
            let mut doom_abort: Option<(&str, u32)> = None;
            for call in outcome.tool_calls.iter() {
                let streak = doom.observe(call);
                doom_streaks.push(streak);
                if streak >= DOOM_ABORT && doom_abort.is_none() {
                    doom_abort = Some((call.name.as_str(), streak));
                }
            }
            for (index, call) in outcome.tool_calls.iter().enumerate() {
                if doom_streaks[index] >= DOOM_BLOCK {
                    results[index] = Some(Err(doom_block_error(&call.name, doom_streaks[index])));
                    continue;
                }
                if call.name == "spawn_subagent" {
                    let Some(squad) = squad.as_ref() else {
                        results[index] = Some(Err("本子智能体不是军团指挥，不能派发".into()));
                        continue;
                    };
                    let member_type = serde_json::from_str::<serde_json::Value>(&call.arguments)
                        .ok()
                        .and_then(|value| value.get("subagent_type").and_then(|v| v.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "explore".into());
                    let Some(member) = squad.members.iter().find(|member| member.agent == member_type) else {
                        results[index] = Some(Err(format!(
                            "{member_type} 不是军团成员；可派：{}",
                            squad.members.iter().map(|m| m.agent.as_str()).collect::<Vec<_>>().join("、")
                        )));
                        continue;
                    };
                    if member_spawns.len() >= resolve_max_parallel(&defaults) {
                        results[index] = Some(Err(format!("同一轮最多运行 {} 个子智能体", resolve_max_parallel(&defaults))));
                        continue;
                    }
                    if !try_spawn_budget(&spawn_budget) {
                        results[index] = Some(Err("本 run 子智能体总额度已用完".into()));
                        continue;
                    }
                    let member_id = next_subagent_id(run_id);
                    let member_desc = serde_json::from_str::<serde_json::Value>(&call.arguments)
                        .ok()
                        .and_then(|value| value.get("description").and_then(|v| v.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "军团成员任务".into());
                    let spawn_detail = format!("{member_type}/{member_desc}");
                    kernel.emit_spawn(run_id, &member_id, ToolPhase::Started, &spawn_detail).await;
                    kernel
                        .record_spawn(
                            run_id,
                            &member_id,
                            &spawn_detail,
                            &crate::services::tool_router::ToolRouter::history_target("spawn_subagent", &call.arguments),
                        )
                        .await;
                    member_spawns.push((index, member_id, member.clone(), call.arguments.clone(), member_desc));
                } else if allowed_names.iter().any(|name| name == &call.name) {
                    runnable.push((call.name.clone(), call.arguments.clone()));
                    runnable_indices.push(index);
                } else {
                    results[index] = Some(Err(format!("子智能体不允许调用工具：{}", call.name)));
                }
            }
            tool_count += runnable.len() + member_spawns.len();
            // 成员派发：Box::pin 递归 run_subagent（成员级+军团级授权并入），
            // resolve_squad=false——成员只引用 agent 定义，禁止军团套军团。
            if !member_spawns.is_empty() {
                let futures: Vec<_> = member_spawns
                    .iter()
                    .map(|(_, member_id, member, arguments, _)| {
                        let mut grants = member.grants.clone();
                        if let Some(squad) = &squad {
                            grants.extend(squad.squad_grants.clone());
                        }
                        Box::pin(run_subagent(
                            kernel.clone(),
                            config,
                            session_id,
                            run_id,
                            member_id.clone(),
                            arguments,
                            cancel.clone(),
                            spawn_budget.clone(),
                            false,
                            grants,
                        ))
                    })
                    .collect();
                let spawned = join_all(futures).await;
                for ((index, member_id, member, _, member_desc), result) in member_spawns.into_iter().zip(spawned) {
                    let detail = match &result {
                        Ok(text) => text.chars().take(256).collect::<String>(),
                        Err(error) => error.clone(),
                    };
                    kernel
                        .emit_spawn(
                            run_id,
                            &member_id,
                            if result.is_ok() { ToolPhase::Finished } else { ToolPhase::Failed },
                            &format!("{member_desc} · {detail}"),
                        )
                        .await;
                    // 必选成员核销：派发成功即计（与装配核销同口径）。
                    if result.is_ok() {
                        pending_members.retain(|name| name != &member.agent);
                    }
                    results[index] = Some(result.map(|text| (text, Vec::new())));
                }
            }
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
            for (index, (call, result)) in outcome.tool_calls.iter().zip(results).enumerate() {
                let (mut text, result_images, is_error) = match result.unwrap_or_else(|| Err("missing tool result".into())) {
                    Ok((value, images)) => (bounded_result_text(&value), images, false),
                    Err(error) => (format!("子智能体工具失败：{error}"), Vec::new(), true),
                };
                if doom_streaks[index] == DOOM_WARN && !is_error {
                    text.push_str(&doom_warn_note(doom_streaks[index]));
                }
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: text,
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.name.clone()),
                    tool_calls: None,
                    images: if result_images.is_empty() { None } else { Some(result_images) },
                    thinking: None,
                    is_error,
                });
            }
            // 停滞硬终止：子智能体带诊断收尾，父级能读到停滞原因。
            if let Some((name, streak)) = doom_abort {
                let content = format!("子智能体检测到工具调用停滞：{name} 连续 {streak} 次完全相同，已终止");
                kernel
                    .audit_turn_anomaly(
                        run_id,
                        format!("subagent={} doom_abort tool={} streak={}", subagent_id, name, streak),
                    )
                    .await;
                return ChildOutcome { status: SubagentStatus::Completed, content, error: None, input_tokens, output_tokens, reasoning_tokens, first_token_ms, elapsed_ms: started.elapsed().as_millis() as u64, decode_ms, decode_tokens, last_input_tokens, tool_count };
            }
            // 必选装配/必派成员逐轮跟进：只要还没真实完成，每一轮都提醒——
            // 直到核销或步数/预算耗尽（耗尽时在结果尾部标注违例）。
            if !pending_grants.is_empty() || !pending_members.is_empty() {
                messages.push(ChatMessage::text("user", pending_reminder(&pending_grants, &pending_members)));
            }
        }
        elapsed_ms = started.elapsed().as_millis() as u64;
        ChildOutcome {
            status: SubagentStatus::Completed,
            content: notes,
            error: Some(format!("达到轮次上限（{steps} 轮），已返回部分结论")),
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
    if !pending_members.is_empty() {
        summary.push_str(&format!(
            "\n【编排违例】军团指挥未派发必选成员：{}——如结论依赖这些视角，请补齐后重派。",
            pending_members.join("、"),
        ));
    }
    // 截断标注透传父模型：耗尽/收尾的调研不是完整证据——此前只进 UI
    // 视图，父级会把部分结论当完整调研采信。
    if status == SubagentStatus::Completed {
        if let Some(note) = &error {
            summary.push_str(&format!("\n【调研受限】{note}——结论基于已收集证据，可能不完整。"));
        }
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
        // 全 run 子智能体总额度（父级 spawn、军团成员、孙级同池扣减）——防扇出。
        let subagent_defaults = kernel.agent_defaults()?;
        let spawn_budget = Arc::new(AtomicUsize::new(resolve_max_total(&subagent_defaults)));
        // 事后验收追问上限：模型无视欠账连续收尾时避免死循环，2 次后放行。
        // 会话级欠账仍在——下一轮首个实质调用照样被 Layer-B 拦。
        let mut post_nudges = 0u32;
        const POST_NUDGE_LIMIT: u32 = 2;
        // 空轮追问上限：provider 完成一轮却零正文零可解析调用（网关丢弃
        // 工具调用等瞬时异常）时追问自愈；耗尽后以可见标记收尾。
        let mut empty_nudges = 0u32;
        const EMPTY_NUDGE_LIMIT: u32 = 2;
        // 停滞检测：同名同参调用连续出现时软警告→拦截→硬终止，
        // 防止模型对同一失败调用原样重试烧完整个 max_rounds 预算。
        let mut doom = DoomGuard::default();
        let mut doom_block_audited = false;
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
                let truncated = outcome
                    .finish_reason
                    .as_deref()
                    .is_some_and(|reason| matches!(reason, "length" | "max_tokens" | "model_context_window_exceeded"));
                if truncated {
                    kernel.token(run.id, "\n\n[输出达到生成长度上限（max_tokens），回复已被截断]".into()).await;
                }
                // 空轮/丢调用异常：完成一轮却零可见正文，或 provider 报告了
                // 工具调用（finish_reason=tool_calls / 碎片被丢弃）却一个都
                // 没解析出来——先追问给模型自愈机会，耗尽后以用户可见标记
                // 收尾，绝不静默完成。
                let calls_lost = outcome.dropped_tool_calls > 0
                    || outcome.finish_reason.as_deref() == Some("tool_calls");
                if !truncated && (outcome.content.trim().is_empty() || calls_lost) {
                    if empty_nudges < EMPTY_NUDGE_LIMIT {
                        empty_nudges += 1;
                        kernel
                            .audit_turn_anomaly(
                                run.id,
                                format!(
                                    "finish_reason={} content_bytes={} dropped_tool_calls={}",
                                    outcome.finish_reason.as_deref().unwrap_or("none"),
                                    outcome.content.len(),
                                    outcome.dropped_tool_calls
                                ),
                            )
                            .await;
                        messages.push(ChatMessage::text(
                            "user",
                            "【系统提示】上一轮没有产生任何可见回复或可执行的工具调用。请直接以文本给出答复；如需调用工具，请重新发起。",
                        ));
                        continue;
                    }
                    let dropped_note = if outcome.dropped_tool_calls > 0 {
                        format!("，丢弃 {} 个工具调用碎片", outcome.dropped_tool_calls)
                    } else {
                        String::new()
                    };
                    kernel
                        .token(
                            run.id,
                            format!(
                                "\n\n[模型本轮未产生可见输出或可执行的工具调用（finish_reason: {}{}）。可能是网关格式异常——请重试或更换模型/网关]",
                                outcome.finish_reason.as_deref().unwrap_or("unknown"),
                                dropped_note
                            ),
                        )
                        .await;
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
            let mut results: Vec<Option<AppResult<ToolPayload>>> = vec![None; outcome.tool_calls.len()];
            let mut normal_calls: Vec<(String, String)> = Vec::new();
            let mut normal_indices: Vec<usize> = Vec::new();
            let mut spawns = Vec::new();
            let mut asks = Vec::new();
            // 特殊通道（spawn/ask）的架构放行附言——跟结果文本一起回喂。
            let mut spawn_reminders: HashMap<usize, Option<String>> = HashMap::new();
            // 停滞检测 pre-pass：按序计数同名同参连续调用。
            // Warn 照常执行（结果尾部附警告）；Block 不执行改喂合成错误；
            // Abort 记录后在本轮消息落库后硬终止。
            let mut doom_streaks: Vec<u32> = Vec::with_capacity(outcome.tool_calls.len());
            let mut doom_abort: Option<(&str, u32)> = None;
            for call in outcome.tool_calls.iter() {
                let streak = doom.observe(call);
                doom_streaks.push(streak);
                if streak >= DOOM_ABORT && doom_abort.is_none() {
                    doom_abort = Some((call.name.as_str(), streak));
                }
            }
            for (index, call) in outcome.tool_calls.iter().enumerate() {
                if doom_streaks[index] >= DOOM_BLOCK {
                    results[index] = Some(Err(doom_block_error(&call.name, doom_streaks[index])));
                    if !doom_block_audited {
                        doom_block_audited = true;
                        kernel
                            .audit_turn_anomaly(
                                run.id,
                                format!("doom_block tool={} streak={}", call.name, doom_streaks[index]),
                            )
                            .await;
                    }
                    continue;
                }
                if call.name == "spawn_subagent" {
                    // 特殊通道也过架构求值——`any`/`prompt` 链、派发义务
                    // （spawn 自身满足不互锁）与普通调用同语义。
                    let check = kernel
                        .architecture_check_special(run.id, run.session_id, "spawn_subagent", &call.arguments, &run.prompt)
                        .await;
                    if let Err(reason) = check {
                        results[index] = Some(Err(reason));
                        continue;
                    }
                    spawn_reminders.insert(index, check.unwrap_or_default());
                    if spawns.len() >= resolve_max_parallel(&subagent_defaults) {
                        results[index] = Some(Err(format!("同一轮最多运行 {} 个子智能体", resolve_max_parallel(&subagent_defaults))));
                        continue;
                    }
                    let description = serde_json::from_str::<serde_json::Value>(&call.arguments).ok()
                        .and_then(|value| value.get("description").and_then(|item| item.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "研究子任务".into());
                    let subagent_id = next_subagent_id(run.id);
                    kernel.emit_spawn(run.id, &subagent_id, ToolPhase::Started, &description).await;
                    kernel
                        .record_spawn(
                            run.id,
                            &subagent_id,
                            &description,
                            &crate::services::tool_router::ToolRouter::history_target("spawn_subagent", &call.arguments),
                        )
                        .await;
                    spawns.push((index, subagent_id, call.arguments.clone(), description));
                } else if call.name == "ask_user" {
                    // 与 spawn 同理过架构求值——`plan` 触发器覆盖 ask_user。
                    match kernel
                        .architecture_check_special(run.id, run.session_id, "ask_user", &call.arguments, &run.prompt)
                        .await
                    {
                        Ok(reminder) => {
                            spawn_reminders.insert(index, reminder);
                            asks.push((index, call.arguments.clone()));
                        }
                        Err(reason) => results[index] = Some(Err(reason)),
                    }
                } else {
                    normal_calls.push((call.name.clone(), call.arguments.clone()));
                    normal_indices.push(index);
                }
            }
            for (position, (index, arguments)) in asks.into_iter().enumerate() {
                let result = if position == 0 {
                    kernel.ask_user(run.id, &arguments, cancel.clone()).await.map(|text| {
                        let reminder = spawn_reminders.remove(&index).flatten();
                        (reminder.map(|r| format!("{text}\n\n{r}")).unwrap_or(text), Vec::new())
                    })
                } else {
                    Err("同一轮最多提出 1 个问题".into())
                };
                results[index] = Some(result);
            }
            let spawned = join_all(spawns.iter().map(|(_, subagent_id, arguments, _)| {
                run_subagent(
                    kernel.clone(),
                    &config,
                    run.session_id,
                    run.id,
                    subagent_id.clone(),
                    arguments,
                    cancel.clone(),
                    spawn_budget.clone(),
                    true,
                    Vec::new(),
                )
            })).await;
            for ((index, subagent_id, _, description), result) in spawns.into_iter().zip(spawned) {
                let detail = match &result { Ok(text) => text.chars().take(512).collect::<String>(), Err(error) => error.clone() };
                kernel.emit_spawn(run.id, &subagent_id, if result.is_ok() { ToolPhase::Finished } else { ToolPhase::Failed }, &format!("{description} · {detail}")).await;
                let reminder = spawn_reminders.remove(&index).flatten();
                results[index] = Some(result.map(|text| {
                    (reminder.map(|r| format!("{text}\n\n{r}")).unwrap_or(text), Vec::new())
                }));
            }
            if !normal_calls.is_empty() {
                let normal_results = kernel.execute_tool_calls(run.id, &normal_calls).await;
                for (index, result) in normal_indices.into_iter().zip(normal_results) {
                    results[index] = Some(result);
                }
            }
            for (index, (call, result)) in outcome.tool_calls.iter().zip(results).enumerate() {
                let (mut text, result_images, is_error) = match result.unwrap_or_else(|| Err("missing tool result".into())) {
                    Ok((text, images)) => (text, images, false),
                    Err(error) => (format!("工具调用失败：{error}"), Vec::new(), true),
                };
                if doom_streaks[index] == DOOM_WARN && !is_error {
                    text.push_str(&doom_warn_note(doom_streaks[index]));
                }
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: bounded_result_text(&text),
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.name.clone()),
                    tool_calls: None,
                    images: if result_images.is_empty() { None } else { Some(result_images) },
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
            // 停滞硬终止：模型无视警告与拦截仍原样重发同一调用——
            // 可见标记 + 审计，run 完成（停滞上下文已落库，「继续」可见）。
            if let Some((name, streak)) = doom_abort {
                kernel
                    .audit_turn_anomaly(run.id, format!("doom_abort tool={name} streak={streak}"))
                    .await;
                kernel.token(run.id, doom_abort_marker(name, streak)).await;
                return Ok(true);
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
        vision_capable: false,
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
    fn doom_guard_counts_only_consecutive_identical_calls() {
        let call = |name: &str, args: &str| ToolCallSpec {
            id: "c".into(),
            name: name.into(),
            arguments: args.into(),
        };
        let mut doom = DoomGuard::default();
        assert_eq!(doom.observe(&call("list_dir", r#"{"path":"."}"#)), 1);
        assert_eq!(doom.observe(&call("list_dir", r#"{"path":"."}"#)), 2);
        assert_eq!(doom.observe(&call("list_dir", r#"{"path":"."}"#)), 3);
        // 换参数即重置——不跨不同调用累加。
        assert_eq!(doom.observe(&call("list_dir", r#"{"path":"src"}"#)), 1);
        // 换工具同样重置。
        assert_eq!(doom.observe(&call("read_file", r#"{"path":"."}"#)), 1);
        // 交错模式（A,B,A,B）永不触发。
        for _ in 0..10 {
            doom.observe(&call("a", "{}"));
            assert_eq!(doom.observe(&call("b", "{}")), 1);
        }
        // 空白参数也算有效签名。
        assert_eq!(doom.observe(&call("pwd", "")), 1);
        assert_eq!(doom.observe(&call("pwd", "")), 2);
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
            run_images: Default::default(),
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
    fn current_turn_images_reach_the_conversation() {
        let (mut detail, current) = fixture(RunStatus::Completed, "done");
        detail.run_images.insert(2, vec!["data:image/webp;base64,CURRENT".into()]);
        let messages = conversation(&detail, &current, None);
        let last = messages.last().unwrap();
        assert_eq!(last.role, "user");
        assert_eq!(last.images.as_deref(), Some(&["data:image/webp;base64,CURRENT".to_string()][..]));
    }

    #[test]
    fn replay_keeps_only_the_two_most_recent_image_turns() {
        let (mut detail, _current) = fixture(RunStatus::Completed, "done");
        // 三个含图 prior（id 1/2/3）+ 当前轮 id 5：回放预算只够最近 2 个保留图片。
        let priors: Vec<RunView> = [
            (1, "最早一轮", 1),
            (2, "中间一轮", 2),
            (3, "最近一轮", 3),
        ]
        .into_iter()
        .map(|(id, prompt, created_at)| RunView {
            id,
            session_id: 7,
            prompt: prompt.into(),
            mode: Mode::Code,
            status: RunStatus::Completed,
            output: "answer".into(),
            error: None,
            created_at,
        })
        .collect();
        detail.runs = priors;
        for id in [1, 2, 3] {
            detail.run_images.insert(id, vec![format!("data:image/webp;base64,IMG{id}")]);
        }
        let current = RunView {
            id: 5,
            session_id: 7,
            prompt: "继续".into(),
            mode: Mode::Code,
            status: RunStatus::Running,
            output: String::new(),
            error: None,
            created_at: 5,
        };
        let messages = conversation(&detail, &current, None);
        let find = |needle: &str| messages.iter().find(|m| m.content.contains(needle)).cloned();
        let oldest = find("最早一轮").expect("最早一轮应在回放中");
        let middle = find("中间一轮").expect("中间一轮应在回放中");
        let newest = find("最近一轮").expect("最近一轮应在回放中");
        assert!(oldest.images.is_none(), "最老含图轮不应回放图片");
        assert!(oldest.content.contains("未回放"), "最老轮应有未回放提示: {}", oldest.content);
        assert_eq!(middle.images.as_deref(), Some(&["data:image/webp;base64,IMG2".to_string()][..]));
        assert_eq!(newest.images.as_deref(), Some(&["data:image/webp;base64,IMG3".to_string()][..]));
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
        let empty = crate::domain::SubagentDefaults::default();
        assert_eq!(subagent_tool_budget(&definition, &empty), crate::domain::DEFAULT_SUBAGENT_TOOL_BUDGET as usize);
        definition.tool_budget = Some(0);
        assert_eq!(subagent_tool_budget(&definition, &empty), 1);
        definition.tool_budget = Some(9999);
        assert_eq!(subagent_tool_budget(&definition, &empty), crate::domain::MAX_SUBAGENT_TOOL_BUDGET as usize);
        definition.tool_budget = Some(60);
        assert_eq!(subagent_tool_budget(&definition, &empty), 60);
    }

    #[test]
    fn subagent_defaults_form_the_middle_fallback_tier() {
        let mut definition = crate::domain::AgentDefinition::builtin("explore").unwrap();
        let defaults = crate::domain::SubagentDefaults { model: None, steps: Some(12), tool_budget: Some(42), max_parallel: Some(3), max_total: Some(9) };
        // 未设置字段回退到全局默认；已设置字段（steps=Some(8)）agent 胜出。
        assert_eq!(resolve_steps(&definition, &defaults), 8);
        assert_eq!(subagent_tool_budget(&definition, &defaults), 42);
        definition.steps = None;
        assert_eq!(resolve_steps(&definition, &defaults), 12);
        assert_eq!(resolve_max_parallel(&defaults), 3);
        assert_eq!(resolve_max_total(&defaults), 9);
        // 全局默认也未设置时回退内置；env 仅作未设置项的兜底。
        let empty = crate::domain::SubagentDefaults::default();
        assert_eq!(resolve_max_parallel(&empty), 2);
        assert_eq!(resolve_max_total(&empty), 8);
        // 文件值超界时 clamp，而不是透传。
        let wild = crate::domain::SubagentDefaults { max_parallel: Some(99), max_total: Some(0), ..Default::default() };
        assert_eq!(resolve_max_parallel(&wild), 4);
        assert_eq!(resolve_max_total(&wild), 1);
    }
}
