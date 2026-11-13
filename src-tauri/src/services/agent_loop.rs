//! Agent loop (ReAct): think → call tools → observe → answer. The LLM
//! adapter streams normalized `StreamEvent`s over a channel; this service
//! relays them to the kernel as typed domain events, executes tool calls via
//! the kernel's authorized path, and feeds results back for the next turn.
use std::{sync::Arc, time::{Duration, Instant}};

use tokio::sync::{mpsc, watch};

use crate::{
    adapters::llm::{ChatConfig, LlmProvider, StreamEvent, TurnOutcome},
    domain::{
        AppResult, ChatMessage, Mode, RunMetrics, RunStatus, RunView, SessionDetail,
        ToolSpec,
    },
    services::kernel::Kernel,
};

/// Safety cap for one run; can be raised with MAESTRO_MAX_TOOL_ROUNDS.
fn max_tool_rounds() -> usize {
    std::env::var("MAESTRO_MAX_TOOL_ROUNDS").ok().and_then(|value| value.parse::<usize>().ok()).filter(|value| *value >= 8).unwrap_or(64)
}
const MAX_TOOL_RESULT_BYTES: usize = 65536;
const METRICS_REPORT_INTERVAL: Duration = Duration::from_millis(200);

/// Scoped reads only reach the selected workspace. Tell the model up front so
/// it explains the boundary instead of burning turns probing outside paths.
const SANDBOX_HINT: &str = "如果任务需要工作区之外的文件（例如用户提到的其他仓库路径），不要尝试绕过沙盒读取；请明确告知用户该路径不在工作区，并请用户将其复制或移动到工作区内后再继续。";

/// Task-tracker discipline: the model must publish a plan before multi-step
/// work and refresh it after every finished step, otherwise the UI panel has
/// nothing to render.
const TASK_PLAN_HINT: &str = "在执行涉及多个文件或步骤的复杂任务前，必须先调用 update_plan 列出计划（每个步骤包含 id、description、status；初始为 pending，当前要做的步骤为 in_progress）。在后续执行中，每完成一步，必须重新调用 update_plan 提交完整步骤列表并更新各步状态；全部完成时所有步骤都应为 completed。简单的单步任务可以跳过。";

/// Conversation assembly: system policy for the run's mode plus prior
/// completed turns of the same session within a byte budget.
pub fn conversation(detail: &SessionDetail, run: &RunView) -> Vec<ChatMessage> {
    let policy = match run.mode {
        Mode::Plan => "你处于只读计划模式。仅可使用列表中提供的只读工具进行分析；写操作与命令执行不会出现在工具列表中，也不得声称已应用变更。",
        Mode::Code => "你处于 Code 模式。读操作和写文件/补丁直接执行；只有危险命令会挂起等待批准，安全命令直接执行——以工具结果为准，不得声称已应用未成功的修改。",
        Mode::Confirm => "你处于 Confirm 模式。每个工具调用（含只读）都会挂起等待用户逐项确认后才执行；以工具结果为准，不得声称已执行未经确认的操作。",
        Mode::Execute => "你处于历史 execute 模式（遗留记录）。仅可使用实际开放且授权的能力。",
        Mode::Auto => "你处于 Auto 策略。已开放的工具调用会直接执行；以工具结果为准，不得声称拥有未提供的能力。",
    };
    let mut messages = vec![ChatMessage::text(
        "system",
        format!("{policy}\n{SANDBOX_HINT}\n{TASK_PLAN_HINT}"),
    )];
    let prior: Vec<_> = detail
        .runs
        .iter()
        .filter(|item| {
            item.id < run.id
                && item.session_id == run.session_id
                && item.status == RunStatus::Completed
                && !item.output.is_empty()
        })
        .collect();
    let mut budget = 131072usize;
    let mut included = Vec::new();
    for item in prior.into_iter().rev().take(32) {
        let size = item.prompt.len() + item.output.len();
        if size > budget {
            break;
        }
        budget -= size;
        included.push(item);
    }
    for item in included.into_iter().rev() {
        messages.push(ChatMessage::text("user", item.prompt.clone()));
        messages.push(ChatMessage::text("assistant", item.output.clone()));
    }
    messages.push(ChatMessage::text("user", run.prompt.clone()));
    messages
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
            let started = Instant::now();
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
                elapsed_ms: 0,
                first_token_ms: None,
            };
            // Drain relay: drop any buffered events the instant the run is
            // cancelled — this is what makes "stop" actually stop the stream.
            loop {
                let event = tokio::select! {
                    event = rx.recv() => event,
                    _ = relay_cancel.changed() => break,
                };
                let Some(event) = event else { break };
                match event {
                    StreamEvent::Reasoning(token) => kernel.reasoning_token(run_id, token).await,
                    StreamEvent::Token(token) => {
                        if first_token_ms.is_none() {
                            first_token_ms = Some(started.elapsed().as_millis() as u64);
                            metrics.first_token_ms = first_token_ms;
                        }
                        kernel.token(run_id, token).await;
                        if reported.elapsed() >= METRICS_REPORT_INTERVAL {
                            metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                            let _ = kernel.report_metrics(metrics.clone(), false).await;
                            reported = Instant::now();
                        }
                    }
                    StreamEvent::Usage { input, output, cache_hit } => {
                        metrics.input_tokens = input.or(metrics.input_tokens);
                        metrics.output_tokens = output.or(metrics.output_tokens);
                        metrics.cache_hit_tokens = cache_hit.or(metrics.cache_hit_tokens);
                    }
                    StreamEvent::Done => {
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        let _ = kernel.report_metrics(metrics.clone(), true).await;
                    }
                }
            }
        })
    };
    let outcome = provider.clone().stream(config, messages, tools, cancel, tx).await;
    let _ = relay.await;
    outcome
}

/// Read-only research subagent. It never writes files or runs commands; its
/// final text is returned to the parent model as a tool result.
async fn run_subagent(
    kernel: Arc<Kernel>,
    config: &ChatConfig,
    arguments_json: &str,
    mut cancel: watch::Receiver<bool>,
    run_id: i64,
) -> AppResult<String> {
    let arguments: serde_json::Value =
        serde_json::from_str(arguments_json).unwrap_or_else(|_| serde_json::json!({}));
    let description = arguments.get("description").and_then(|v| v.as_str()).unwrap_or("研究子任务").trim().to_string();
    let prompt = arguments.get("prompt").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if prompt.is_empty() || prompt.len() > 8192 {
        return Err("spawn_subagent 需要 1–8192 字节的 prompt".into());
    }
    let provider = kernel.llm_provider(config);
    let tools: Vec<_> = kernel
        .agent_tools(Mode::Plan)
        .await
        .into_iter()
        .filter(|tool| tool.name != "spawn_subagent")
        .collect();
    let mut messages = vec![
        ChatMessage::text("system", "你是只读研究子智能体。只使用提供的只读工具分析，不写文件、不执行命令；完成后用简洁中文给出结论、证据路径和风险。"),
        ChatMessage::text("user", prompt),
    ];
    for _ in 0..8 {
        if *cancel.borrow() {
            return Err("子智能体已取消".into());
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let outcome = provider
            .clone()
            .stream(config, &messages, &tools, &mut cancel, tx)
            .await?;
        let mut text = String::new();
        while let Ok(event) = rx.try_recv() {
            if let StreamEvent::Token(token) = event {
                text.push_str(&token);
            }
        }
        if !outcome.done {
            return Err("子智能体已取消".into());
        }
        if outcome.tool_calls.is_empty() {
            return Ok(format!("【子智能体 {description}】
{}", text.trim()));
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: text,
            tool_calls: Some(outcome.tool_calls.clone()),
            tool_call_id: None,
            name: None,
            images: None,
        });
        let calls: Vec<(String, String)> = outcome
            .tool_calls
            .iter()
            .map(|call| (call.name.clone(), call.arguments.clone()))
            .collect();
        let results = kernel.execute_tool_calls(run_id, &calls).await;
        for (call, result) in outcome.tool_calls.iter().zip(results) {
            messages.push(ChatMessage {
                role: "tool".into(),
                content: match result {
                    Ok(value) => value.chars().take(MAX_TOOL_RESULT_BYTES).collect(),
                    Err(error) => format!("子智能体工具失败：{error}"),
                },
                tool_call_id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                tool_calls: None,
                images: None,
            });
        }
    }
    Ok(format!("【子智能体 {description}】达到工具轮次上限，未完成全部研究"))
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
        let mut messages = conversation(&detail, &run);
        let tools = kernel.agent_tools(run.mode).await;
        let provider = kernel.llm_provider(&config);
        let max_rounds = max_tool_rounds();
        for _round in 0..max_rounds {
            if kernel.is_cancelled(run.id).await {
                return Ok(false);
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
                ) => outcome?,
            };
            if !outcome.done {
                return Ok(false);
            }
            if outcome.tool_calls.is_empty() {
                return Ok(true);
            }
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(outcome.tool_calls.clone()),
                tool_call_id: None,
                name: None,
                images: None,
            });
            // Authorization, audit and tool_call events all happen inside the
            // kernel; suspended calls share ONE approval for the whole turn.
            let mut results: Vec<Option<AppResult<String>>> = vec![None; outcome.tool_calls.len()];
            let mut normal_calls: Vec<(String, String)> = Vec::new();
            let mut normal_indices: Vec<usize> = Vec::new();
            for (index, call) in outcome.tool_calls.iter().enumerate() {
                if call.name == "spawn_subagent" {
                    results[index] = Some(
                        run_subagent(kernel.clone(), &config, &call.arguments, cancel.clone(), run.id).await,
                    );
                } else {
                    normal_calls.push((call.name.clone(), call.arguments.clone()));
                    normal_indices.push(index);
                }
            }
            if !normal_calls.is_empty() {
                let normal_results = kernel.execute_tool_calls(run.id, &normal_calls).await;
                for (index, result) in normal_indices.into_iter().zip(normal_results) {
                    results[index] = Some(result);
                }
            }
            for (call, result) in outcome.tool_calls.iter().zip(results) {
                let text = match result.unwrap_or_else(|| Err("missing tool result".into())) {
                    Ok(text) => text,
                    Err(error) => format!("工具调用失败：{error}"),
                };
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: text.chars().take(MAX_TOOL_RESULT_BYTES).collect(),
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.name.clone()),
                    tool_calls: None,
                    images: None,
                });
            }
        }
        // Tool budget exhausted — give the model one final no-tools turn to
        // summarize what it already gathered instead of failing the run.
        messages.push(ChatMessage::text(
            "user",
            &format!("本轮工具调用达到上限（{max_rounds} 轮），任务可能未完成。请基于已收集的信息总结，并提示用户发送「继续」开始新一轮；本轮不要再调用工具。"),
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
