pub mod adapters;
mod commands;
pub mod domain;
pub mod services;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::{Emitter, Manager};

use adapters::fs::workspace::WorkspaceService;
use domain::KernelEvent;
use services::kernel::{Kernel, EVENT_NAME};

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
                if let Err(error) = handle.emit_to("main", EVENT_NAME, &event) {
                    eprintln!("Cannot emit kernel event: {error}");
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
            app.manage(kernel);
            app.manage(WorkspaceService::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::kernel::get_snapshot,
            commands::activity::activity_snapshot,
            commands::activity::run_change_summary,
            commands::activity::run_changes,
            commands::activity::preview_revert_run_changes,
            commands::activity::revert_run_changes,
            commands::kernel::set_mode,
            commands::kernel::cancel_agent,
            commands::kernel::abort_chat,
            commands::kernel::resolve_tool_approval,
            commands::kernel::list_runs,
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
