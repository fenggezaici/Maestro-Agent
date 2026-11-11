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

const MAX_TOOL_ROUNDS: usize = 32;
const MAX_TOOL_RESULT_BYTES: usize = 65536;
const METRICS_REPORT_INTERVAL: Duration = Duration::from_millis(200);

/// Conversation assembly: system policy for the run's mode plus prior
/// completed turns of the same session within a byte budget.
pub fn conversation(detail: &SessionDetail, run: &RunView) -> Vec<ChatMessage> {
    let policy = match run.mode {
        Mode::Plan => "你处于只读计划模式。仅可使用列表中提供的只读工具进行分析；写操作与命令执行不会出现在工具列表中，也不得声称已应用变更。",
        Mode::Code => "你处于 Code 模式。读操作直接执行；写文件与命令类工具调用会挂起并展示给用户，经批准后才真实执行——你收到的工具结果才是真实执行结果，不得声称已应用未被批准的修改。",
        Mode::Confirm => "你处于 Confirm 模式。每个工具调用（含只读）都会挂起等待用户逐项确认后才执行；以工具结果为准，不得声称已执行未经确认的操作。",
        Mode::Execute => "你处于历史 execute 模式（遗留记录）。仅可使用实际开放且授权的能力。",
        Mode::Auto => "你处于 Auto 策略。已开放的工具调用会直接执行；以工具结果为准，不得声称拥有未提供的能力。",
    };
    let mut messages = vec![ChatMessage::text("system", policy)];
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

pub(crate) async fn run(
    kernel: Arc<Kernel>,
    run: RunView,
    config: ChatConfig,
    mut cancel: watch::Receiver<bool>,
) {
    let result: AppResult<bool> = async {
        if *cancel.borrow() {
            return Ok(false);
        }
        let detail = kernel.session_detail(run.session_id).await?;
        let mut messages = conversation(&detail, &run);
        let tools = kernel.agent_tools(run.mode).await;
        let provider = kernel.llm_provider(&config);
        for _round in 0..MAX_TOOL_ROUNDS {
            if *cancel.borrow() {
                return Ok(false);
            }
            let outcome = stream_turn(
                &kernel,
                &provider,
                &config,
                &messages,
                &tools,
                &mut cancel,
                run.id,
            )
            .await?;
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
            for call in &outcome.tool_calls {
                // Authorization, audit and tool_call events all happen inside
                // the kernel; the result text goes back to the model only.
                let result = kernel
                    .execute_model_tool(run.id, &call.name, &call.arguments)
                    .await;
                let text = match result {
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
            "工具调用预算已用尽。请基于已收集的信息直接给出最终结论，不要再调用工具。",
        ));
        let final_turn = stream_turn(
            &kernel,
            &provider,
            &config,
            &messages,
            &[],
            &mut cancel,
            run.id,
        )
        .await?;
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
