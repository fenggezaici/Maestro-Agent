use crate::domain::{AppResult, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    ProcessExecution,
}

/// What the kernel must do with a tool call under the run's mode.
/// Resolved entirely on the backend — never accepted from frontend/model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGate {
    /// Execute immediately.
    Allow,
    /// Suspend the call and wait for an explicit user approval decision.
    RequireApproval,
    /// Refuse outright (reported to the model as a failed tool call).
    Deny,
}

/// Classify a named tool by effect. `mcp__*` tools are classified by the
/// caller (ToolRouter) via the server's own readOnlyHint annotation.
pub fn classify(tool: &str) -> AppResult<ToolEffect> {
    Ok(match tool {
        "read_session" | "list_sessions" | "load_skill" | "list_dir" | "read_file"
        | "update_plan" => ToolEffect::ReadOnly,
        "apply_patch" | "write_file" => ToolEffect::WorkspaceWrite,
        "run_command" => ToolEffect::ProcessExecution,
        _ if tool.starts_with("mcp__") => ToolEffect::ProcessExecution,
        _ => return Err(format!("Unknown or unregistered tool: {tool}")),
    })
}

/// Mode gate applied to a resolved effect; shared by named and MCP tools.
/// - Plan: read-only only — side-effect tools are denied and must also be
///   stripped from the advertised tool list.
/// - Code: reads execute; writes and process execution suspend for approval.
/// - Confirm: every tool call suspends for per-call confirmation.
/// - Auto/Execute: everything allowed.
pub fn gate(mode: Mode, effect: ToolEffect) -> ToolGate {
    match mode {
        Mode::Plan => match effect {
            ToolEffect::ReadOnly => ToolGate::Allow,
            _ => ToolGate::Deny,
        },
        Mode::Code => match effect {
            ToolEffect::ReadOnly => ToolGate::Allow,
            _ => ToolGate::RequireApproval,
        },
        Mode::Confirm => ToolGate::RequireApproval,
        Mode::Auto | Mode::Execute => ToolGate::Allow,
    }
}

/// Legacy helper kept for callers that only need allow/deny semantics:
/// classification plus gate; `RequireApproval` counts as authorized.
pub fn authorize(mode: Mode, tool: &str) -> AppResult<ToolEffect> {
    let effect = classify(tool)?;
    match gate(mode, effect) {
        ToolGate::Deny => Err(format!(
            "Plan Mode blocked tool: {tool}; only read tools are allowed"
        )),
        _ => Ok(effect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_denies_side_effects_and_allows_read_only_work() {
        for tool in ["read_session", "list_sessions", "load_skill", "list_dir", "read_file", "update_plan"] {
            assert_eq!(authorize(Mode::Plan, tool).unwrap(), ToolEffect::ReadOnly);
        }
        for tool in ["apply_patch", "write_file", "run_command"] {
            assert!(authorize(Mode::Plan, tool).is_err());
        }
    }

    #[test]
    fn gate_matrix_covers_every_mode_and_effect() {
        use ToolGate::*;
        for effect in [ToolEffect::ReadOnly, ToolEffect::WorkspaceWrite, ToolEffect::ProcessExecution] {
            assert_eq!(gate(Mode::Confirm, effect), RequireApproval);
            assert_eq!(gate(Mode::Auto, effect), Allow);
            assert_eq!(gate(Mode::Execute, effect), Allow);
        }
        assert_eq!(gate(Mode::Plan, ToolEffect::ReadOnly), Allow);
        assert_eq!(gate(Mode::Plan, ToolEffect::WorkspaceWrite), Deny);
        assert_eq!(gate(Mode::Plan, ToolEffect::ProcessExecution), Deny);
        assert_eq!(gate(Mode::Code, ToolEffect::ReadOnly), Allow);
        assert_eq!(gate(Mode::Code, ToolEffect::WorkspaceWrite), RequireApproval);
        assert_eq!(gate(Mode::Code, ToolEffect::ProcessExecution), RequireApproval);
    }

    #[test]
    fn code_mode_authorizes_but_does_not_run_side_effects_directly() {
        assert_eq!(authorize(Mode::Code, "write_file").unwrap(), ToolEffect::WorkspaceWrite);
        assert_eq!(authorize(Mode::Code, "read_file").unwrap(), ToolEffect::ReadOnly);
        assert_eq!(
            gate(Mode::Code, authorize(Mode::Code, "write_file").unwrap()),
            ToolGate::RequireApproval
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
                "cat_file",
                "shell",
                "read_only_write_file",
                "",
            ] {
                assert!(authorize(mode, tool).is_err());
            }
        }
    }
}
