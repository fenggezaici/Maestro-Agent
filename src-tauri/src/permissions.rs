use crate::model::{AppResult, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    ProcessExecution,
}

// Only these registered, implemented read tools are allowed in Plan Mode.
pub const PLAN_TOOL_ALLOWLIST: &[&str] = &["read_session", "list_sessions", "load_skill"];

/// Classifications are resolved here, never accepted from the frontend/model.
/// `mcp__*` tools are classified by the caller via `check_mode` using the
/// server's own readOnlyHint annotation.
pub fn authorize(mode: Mode, tool: &str) -> AppResult<ToolEffect> {
    let effect = match tool {
        "read_session" | "list_sessions" | "load_skill" => ToolEffect::ReadOnly,
        "apply_patch" | "write_file" => ToolEffect::WorkspaceWrite,
        "run_command" => ToolEffect::ProcessExecution,
        _ if tool.starts_with("mcp__") => ToolEffect::ProcessExecution,
        _ => return Err(format!("Unknown or unregistered tool: {tool}")),
    };
    check_mode(mode, effect, tool)?;
    Ok(effect)
}

/// Mode gate applied to a resolved effect; shared by named and MCP tools.
pub fn check_mode(mode: Mode, effect: ToolEffect, tool: &str) -> AppResult<()> {
    if mode == Mode::Plan && !PLAN_TOOL_ALLOWLIST.contains(&tool) && effect != ToolEffect::ReadOnly {
        return Err(format!(
            "Plan Mode blocked tool: {tool}; only read tools are allowed"
        ));
    }
    if mode == Mode::Confirm {
        return Err(
            "Confirm mode requires per-tool confirmation; no confirmation executor is enabled"
                .into(),
        );
    }
    if mode == Mode::Code && effect != ToolEffect::ReadOnly {
        return Err(
            "Code mode requires approval of each concrete diff; writes are not enabled".into(),
        );
    }
    Ok(())
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
        for mode in [
            Mode::Plan,
            Mode::Code,
            Mode::Confirm,
            Mode::Auto,
            Mode::Execute,
        ] {
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

    #[test]
    fn code_requires_diff_approval_and_confirm_blocks_unconfirmed_tools() {
        assert!(authorize(Mode::Code, "read_session").is_ok());
        assert!(authorize(Mode::Code, "write_file").is_err());
        assert!(authorize(Mode::Confirm, "read_session").is_err());
        assert!(authorize(Mode::Auto, "read_session").is_ok());
    }
}
