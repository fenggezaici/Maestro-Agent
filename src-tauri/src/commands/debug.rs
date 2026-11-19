use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::{domain::AppResult, services::kernel::Kernel};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugFakeStream {
    pub session_id: i64,
    pub run_id: i64,
}

/// e2e 注入面：造一条真 running run 并持续注入流式 Content 事件。
/// 只在 debug 构建可用——release 里固定返回错误。
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_start_fake_stream(
    kernel: State<'_, Arc<Kernel>>,
    session_id: Option<i64>,
    seconds: Option<u32>,
) -> AppResult<DebugFakeStream> {
    let (session_id, run_id) = kernel
        .debug_fake_stream(session_id, seconds.unwrap_or(8))
        .await?;
    Ok(DebugFakeStream { session_id, run_id })
}

#[cfg(not(debug_assertions))]
#[tauri::command]
pub async fn debug_start_fake_stream(
    _kernel: State<'_, Arc<Kernel>>,
    _session_id: Option<i64>,
    _seconds: Option<u32>,
) -> AppResult<DebugFakeStream> {
    Err("debug commands are only available in debug builds".into())
}
