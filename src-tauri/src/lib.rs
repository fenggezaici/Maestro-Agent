pub mod adapters;
mod commands;
pub mod domain;
pub mod services;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tauri::{Emitter, Manager};
use tokio::sync::mpsc;

use adapters::fs::workspace::WorkspaceService;
use domain::{EventData, KernelEvent};
use services::kernel::{Kernel, EVENT_NAME};

/// Burst window for coalescing kernel events before they reach the webview.
/// Every `emit_to` costs one JS evaluation on the webview main thread; during
/// streaming, per-token events arrive faster than the UI can consume them and
/// starve `invoke` responses (e.g. file preview reads looked permanently
/// "正在读取文件内容…"). Consecutive same-key stream events merge their token
/// strings so ordering and payload semantics stay identical.
const EVENT_FLUSH_WINDOW: Duration = Duration::from_millis(32);
/// Safety cap so a sustained flood cannot delay the flush loop forever.
const EVENT_BATCH_LIMIT: usize = 512;

/// Merge `next` into `into` when both are the same kind of streamed text for
/// the same target. Returns false for any other pair so callers emit in order.
fn try_merge_stream_event(into: &mut KernelEvent, next: &mut KernelEvent) -> bool {
    let sequence = next.sequence;
    let merged = match (&mut into.data, &mut next.data) {
        (
            EventData::Content { run_id: into_run, token: into_token, .. },
            EventData::Content { run_id, token, .. },
        ) if *into_run == *run_id => {
            into_token.push_str(token);
            true
        }
        (
            EventData::Reasoning { run_id: into_run, token: into_token },
            EventData::Reasoning { run_id, token },
        ) if *into_run == *run_id => {
            into_token.push_str(token);
            true
        }
        (
            EventData::SubagentToken {
                run_id: into_run,
                subagent_id: into_subagent,
                kind: into_kind,
                token: into_token,
            },
            EventData::SubagentToken {
                run_id,
                subagent_id,
                kind,
                token,
            },
        ) if *into_run == *run_id && *into_subagent == *subagent_id && *into_kind == *kind => {
            into_token.push_str(token);
            true
        }
        _ => false,
    };
    if merged {
        into.sequence = sequence;
    }
    merged
}

/// Drain `rx` for up to the burst window, merging adjacent same-key stream
/// events into `out`. Event order is preserved: only adjacent same-key token
/// events collapse into one emission.
async fn collect_events(
    rx: &mut mpsc::UnboundedReceiver<KernelEvent>,
    out: &mut Vec<KernelEvent>,
) {
    let deadline = Instant::now() + EVENT_FLUSH_WINDOW;
    loop {
        if out.len() >= EVENT_BATCH_LIMIT {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(mut next)) => {
                let merged = out
                    .last_mut()
                    .is_some_and(|last| try_merge_stream_event(last, &mut next));
                if !merged {
                    out.push(next);
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
}

pub fn run() {
    let shutdown_started = Arc::new(AtomicBool::new(false));
    let exit_ready = Arc::new(AtomicBool::new(false));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _directory| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            },
        ))
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("maestro.sqlite3");
            let handle = app.handle().clone();
            let (tx, mut rx) = mpsc::unbounded_channel::<KernelEvent>();
            let sink = Arc::new(move |event: KernelEvent| {
                // emit_locked calls this while holding the state lock — the
                // unbounded send never blocks.
                let _ = tx.send(event);
            });
            tauri::async_runtime::spawn(async move {
                let mut batch: Vec<KernelEvent> = Vec::new();
                while let Some(first) = rx.recv().await {
                    batch.push(first);
                    collect_events(&mut rx, &mut batch).await;
                    for event in batch.drain(..) {
                        let _ = handle.emit_to("main", EVENT_NAME, &event);
                    }
                }
            });
            // setup is synchronous; SQLite initialization still runs on its OS thread.
            let maestro_dir = app
                .path()
                .home_dir()
                .unwrap_or_else(|_| app.path().app_data_dir().unwrap())
                .join(".maestro");
            let kernel = tauri::async_runtime::block_on(Kernel::open(path, maestro_dir, sink))
                .map_err(std::io::Error::other)?;
            app.manage(kernel.clone());
            app.manage(WorkspaceService::default());
            // Restore MCP servers in the background: a slow npx download must
            // never delay the window or the first chat turn.
            tauri::async_runtime::spawn(async move {
                kernel.start_enabled_mcp().await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::kernel::get_snapshot,
            commands::activity::activity_snapshot,
            commands::agents::list_agents,
            commands::agents::save_agent,
            commands::agents::delete_agent,
            commands::activity::run_change_summary,
            commands::activity::run_changes,
            commands::activity::preview_revert_run_changes,
            commands::activity::revert_run_changes,
            commands::kernel::set_mode,
            commands::kernel::cancel_agent,
            commands::kernel::abort_chat,
            commands::kernel::resolve_tool_approval,
            commands::kernel::list_approval_rules,
            commands::kernel::revoke_approval_rule,
            commands::credentials::save_provider_credential,
            commands::credentials::provider_credential_status,
            commands::credentials::clear_provider_credential,
            commands::kernel::list_runs,
            commands::kernel::list_subagent_traces,
            commands::kernel::cancel_subagent,
            commands::kernel::answer_user_question,
            commands::kernel::compact_session,
            commands::sessions::create_session,
            commands::sessions::set_session_archived,
            commands::sessions::rename_session,
            commands::sessions::delete_session,
            commands::sessions::list_projects,
            commands::sessions::create_project,
            commands::sessions::set_project_archived,
            commands::sessions::update_project_name,
            commands::sessions::update_project_path,
            commands::sessions::delete_project,
            commands::sessions::list_sessions,
            commands::sessions::get_session,
            commands::sessions::decide_plan,
            commands::workspace::pick_directory,
            commands::workspace::pick_file,
            commands::extensions::scan_skills,
            commands::extensions::create_skill,
            commands::extensions::delete_skill,
            commands::extensions::scan_commands,
            commands::extensions::read_command,
            commands::extensions::import_skill,
            commands::extensions::import_skill_github,
            commands::extensions::list_mcp_servers,
            commands::extensions::add_mcp_server,
            commands::extensions::set_mcp_server_trust,
            commands::extensions::import_mcp_json,
            commands::extensions::start_mcp_server,
            commands::extensions::stop_mcp_server,
            commands::extensions::remove_mcp_server,
            commands::extensions::recommended_skills,
            commands::extensions::recommended_mcps,
            commands::extensions::read_mcp_config,
            commands::kernel::stream_chat,
            commands::models::fetch_models,
            commands::models::model_catalog,
            commands::workspace::get_workspace_info,
            commands::workspace::select_workspace,
            commands::workspace::read_directory_tree,
            commands::workspace::read_workspace_text,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window.app_handle().exit(0);
            }
        })
        .build(tauri::generate_context!())
        .expect("Failed to build Maestro desktop application");

    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if exit_ready.load(Ordering::Acquire) {
                return;
            }
            api.prevent_exit();
            if shutdown_started.swap(true, Ordering::AcqRel) {
                return;
            }
            let kernel = handle.state::<Arc<Kernel>>().inner().clone();
            let handle = handle.clone();
            let exit_ready = exit_ready.clone();
            tauri::async_runtime::spawn(async move {
                let code = match kernel.shutdown().await {
                    Ok(()) => 0,
                    Err(_) => 1,
                };
                exit_ready.store(true, Ordering::Release);
                handle.exit(code);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::SubagentTokenKind;

    fn event(sequence: u64, data: EventData) -> KernelEvent {
        KernelEvent { sequence, data }
    }

    fn content(run_id: i64, token: &str) -> EventData {
        EventData::Content { run_id, session_id: 1, token: token.into() }
    }

    #[test]
    fn adjacent_content_tokens_merge_and_keep_the_later_sequence() {
        let mut first = event(1, content(7, "Hello"));
        assert!(try_merge_stream_event(&mut first, &mut event(2, content(7, " world"))));
        match &first.data {
            EventData::Content { token, run_id, .. } => {
                assert_eq!(token, "Hello world");
                assert_eq!(*run_id, 7);
            }
            other => panic!("expected content, got {other:?}"),
        }
        assert_eq!(first.sequence, 2);
    }

    #[test]
    fn different_targets_and_kinds_do_not_merge() {
        let mut base = event(1, content(7, "a"));
        assert!(!try_merge_stream_event(&mut base.clone(), &mut event(2, content(8, "b"))));
        assert!(!try_merge_stream_event(
            &mut base,
            &mut event(2, EventData::Reasoning { run_id: 7, token: "r".into() }),
        ));
        assert!(!try_merge_stream_event(
            &mut event(
                1,
                EventData::SubagentToken {
                    run_id: 7,
                    subagent_id: "s1".into(),
                    kind: SubagentTokenKind::Content,
                    token: "x".into(),
                },
            ),
            &mut event(
                2,
                EventData::SubagentToken {
                    run_id: 7,
                    subagent_id: "s1".into(),
                    kind: SubagentTokenKind::Reasoning,
                    token: "y".into(),
                },
            ),
        ));
    }

    #[tokio::test]
    async fn collect_events_preserves_order_and_merges_only_adjacent_tokens() {
        let (tx, mut rx) = mpsc::unbounded_channel::<KernelEvent>();
        tx.send(event(1, content(7, "a"))).unwrap();
        tx.send(event(2, content(7, "b"))).unwrap();
        tx.send(event(3, EventData::SessionsChanged)).unwrap();
        tx.send(event(4, content(7, "c"))).unwrap();
        drop(tx);
        let mut out = Vec::new();
        collect_events(&mut rx, &mut out).await;
        assert_eq!(out.len(), 3);
        match &out[0].data {
            EventData::Content { token, .. } => assert_eq!(token, "ab"),
            other => panic!("expected merged content, got {other:?}"),
        }
        assert!(matches!(out[1].data, EventData::SessionsChanged));
        match &out[2].data {
            EventData::Content { token, .. } => assert_eq!(token, "c"),
            other => panic!("expected content, got {other:?}"),
        }
        assert_eq!(out[0].sequence, 2);
        assert_eq!(out[2].sequence, 4);
    }
}
