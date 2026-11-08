mod agent;
mod commands;
mod db;
pub mod kernel;
pub mod llm;
pub mod model;
pub mod permissions;
pub mod workspace;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::{Emitter, Manager};

use kernel::{Kernel, EVENT_NAME};
use model::{EventData, KernelEvent, RunStatus};

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
            let sink = Arc::new(move |event: KernelEvent| {
                if let EventData::StreamToken { run_id, session_id, token } = &event.data {
                    let _ = handle.emit_to("main", "chat-token", serde_json::json!({ "sequence": event.sequence, "runId": run_id, "sessionId": session_id, "token": token }));
                    return;
                }
                if let Err(error) = handle.emit_to("main", EVENT_NAME, &event) {
                    eprintln!("Cannot emit kernel event: {error}");
                }
                match &event.data {
                    EventData::Status { run } if run.status != RunStatus::Running => {
                        if run.status == RunStatus::Failed {
                            let _ = handle.emit_to("main", "chat-error", serde_json::json!({ "sequence": event.sequence, "run": run, "message": run.error.as_deref().unwrap_or("模型请求失败") }));
                        } else {
                            let _ = handle.emit_to("main", "chat-end", serde_json::json!({ "sequence": event.sequence, "run": run }));
                        }
                    }
                    EventData::BackendError { message } => {
                        let _ = handle.emit_to("main", "chat-error", serde_json::json!({ "sequence": event.sequence, "message": message }));
                    }
                    _ => {}
                }
            });
            // setup is synchronous; SQLite initialization still runs on its OS thread.
            let kernel = tauri::async_runtime::block_on(Kernel::open(path, sink))
                .map_err(std::io::Error::other)?;
            app.manage(kernel);
            app.manage(workspace::WorkspaceService::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::set_mode,
            commands::start_agent,
            commands::cancel_agent,
            commands::list_runs,
            commands::create_session,
            commands::list_sessions,
            commands::get_session,
            commands::decide_plan,
            llm::stream_chat,
            workspace::get_workspace_info,
            workspace::select_workspace,
            workspace::read_directory_tree,
            workspace::read_workspace_text,
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
                    Err(error) => {
                        eprintln!("Shutdown failed: {error}");
                        1
                    }
                };
                exit_ready.store(true, Ordering::Release);
                handle.exit(code);
            });
        }
    });
}
