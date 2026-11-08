//! Model orchestration and tools live in Rust. Phase 1 uses a clearly labeled placeholder.
pub(crate) mod placeholder;

use std::{sync::Arc, time::Duration};

use tokio::sync::watch;

use crate::{
    kernel::Kernel,
    llm::{self, ChatConfig},
    model::{RunStatus, RunView},
};

use placeholder::{tokens, PlaceholderModel};

pub(crate) async fn run_model(
    kernel: Arc<Kernel>,
    run: RunView,
    config: ChatConfig,
    mut cancel: watch::Receiver<bool>,
) {
    let result = async {
        if *cancel.borrow() {
            return Ok(false);
        }
        let detail = kernel.session_detail(run.session_id).await?;
        let messages = llm::conversation(&detail, &run);
        llm::stream(&config, messages, &mut cancel, &kernel, run.id).await
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

pub(crate) async fn run(kernel: Arc<Kernel>, run: RunView, mut cancel: watch::Receiver<bool>) {
    if *cancel.borrow() {
        kernel.finish(run.id, RunStatus::Cancelled, None).await;
        return;
    }
    let context = match kernel.dispatch_tool(run.id, "read_session").await {
        Ok(context) => context,
        Err(error) => {
            let status = if *cancel.borrow() {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            };
            kernel.finish(run.id, status, Some(error)).await;
            return;
        }
    };
    let turn = context["runs"].as_array().map_or(1, Vec::len);
    let reply = PlaceholderModel.reply(&run.prompt, turn);
    if !stream(&kernel, run.id, &reply.text, &mut cancel).await {
        kernel.finish(run.id, RunStatus::Cancelled, None).await;
        return;
    }
    // The model proposes a named tool; Rust determines its effect and intercepts it.
    let _ = kernel.dispatch_tool(run.id, reply.proposed_tool).await;
    if let Err(error) = kernel.publish_plan(run.id, reply.plan).await {
        let status = if *cancel.borrow() {
            RunStatus::Cancelled
        } else {
            RunStatus::Failed
        };
        kernel.finish(run.id, status, Some(error)).await;
        return;
    }
    if !stream(
        &kernel,
        run.id,
        "\n\n写入工具已被拦截。请审查下方计划卡片与右侧 Diff，选择批准或拒绝；批准不会自动执行。",
        &mut cancel,
    )
    .await
    {
        kernel.finish(run.id, RunStatus::Cancelled, None).await;
        return;
    }
    kernel.finish(run.id, RunStatus::Completed, None).await;
}

async fn stream(
    kernel: &Kernel,
    run_id: i64,
    text: &str,
    cancel: &mut watch::Receiver<bool>,
) -> bool {
    for token in tokens(text) {
        if *cancel.borrow() {
            return false;
        }
        tokio::select! {
            biased;
            _ = cancel.changed() => {
                return false;
            }
            _ = tokio::time::sleep(Duration::from_millis(16)) => {}
        }
        if *cancel.borrow() {
            return false;
        }
        kernel.token(run_id, token).await;
    }
    true
}
