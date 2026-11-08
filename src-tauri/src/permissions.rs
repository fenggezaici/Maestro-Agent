use crate::model::{AppResult, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    ProcessExecution,
}

// Only these registered, implemented read tools are allowed in Plan Mode.
pub const PLAN_TOOL_ALLOWLIST: &[&str] = &["read_session", "list_sessions"];

/// Classifications are resolved here, never accepted from the frontend/model.
pub fn authorize(mode: Mode, tool: &str) -> AppResult<ToolEffect> {
    let effect = match tool {
        "read_session" | "list_sessions" => ToolEffect::ReadOnly,
        "apply_patch" | "write_file" => ToolEffect::WorkspaceWrite,
        "run_command" => ToolEffect::ProcessExecution,
        _ => return Err(format!("Unknown or unregistered tool: {tool}")),
    };
    if mode == Mode::Plan && !PLAN_TOOL_ALLOWLIST.contains(&tool) {
        return Err(format!(
            "Plan Mode blocked tool: {tool}; only read_session and list_sessions are allowed"
        ));
    }
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_denies_side_effects_and_allows_read_only_work() {
        for tool in PLAN_TOOL_ALLOWLIST {
            assert_eq!(authorize(Mode::Plan, tool).unwrap(), ToolEffect::ReadOnly);
        }
        for tool in ["apply_patch", "write_file", "run_command"] {
            assert!(authorize(Mode::Plan, tool).is_err());
        }
    }

    #[test]
    fn execute_mode_is_an_explicit_opt_in() {
        assert_eq!(
            authorize(Mode::Execute, "apply_patch").unwrap(),
            ToolEffect::WorkspaceWrite
        );
        assert_eq!(
            authorize(Mode::Execute, "run_command").unwrap(),
            ToolEffect::ProcessExecution
        );
    }

    #[test]
    fn unknown_aliases_and_spoofed_read_tools_fail_closed_in_all_modes() {
        for mode in [Mode::Plan, Mode::Execute] {
            for tool in [
                "READ_SESSION",
                "read_session;rm",
                "read_file",
                "shell",
                "read_only_write_file",
                "",
            ] {
                assert!(authorize(mode, tool).is_err());
            }
        }
    }
}
