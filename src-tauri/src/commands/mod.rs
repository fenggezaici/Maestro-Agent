//! IPC layer: every command is a thin pass-through that validates the window,
//! delegates to a service/adapter, and wraps errors. No business logic here.
pub mod extensions;
pub mod kernel;
pub mod models;
pub mod sessions;
pub mod workspace;

use tauri::WebviewWindow;

use crate::domain::AppResult;

// Defense in depth in addition to local-window capabilities; callers supply no paths or SQL.
pub(crate) fn require_main(window: &WebviewWindow) -> AppResult<()> {
    if window.label() != "main" {
        return Err("Only the main window may invoke backend commands".into());
    }
    Ok(())
}
