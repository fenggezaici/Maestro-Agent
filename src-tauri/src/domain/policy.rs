use crate::domain::{command_policy, AppResult, Mode};

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
        // task_stop 只能终止本进程拉起的后台任务，归为只读类（Plan 下也无可停任务）。
        "read_session" | "list_sessions" | "load_skill" | "list_dir" | "read_file"
        | "glob_files" | "grep_files" | "task_output" | "task_stop"
        | "update_plan" | "spawn_subagent" | "ask_user" => ToolEffect::ReadOnly,
        "apply_patch" | "write_file" => ToolEffect::WorkspaceWrite,
        "run_command" => ToolEffect::ProcessExecution,
        _ if tool.starts_with("mcp__") => ToolEffect::ProcessExecution,
        _ => return Err(format!("Unknown or unregistered tool: {tool}")),
    })
}

/// Mode gate applied to a resolved effect; shared by named and MCP tools.
/// - Plan: read-only only.
/// - Code (自主模式): reads and workspace writes execute; commands run unless
///   the risk classifier flags them dangerous (`command_policy::is_dangerous`).
/// - Confirm: every tool call suspends for per-call confirmation.
/// - Auto/Execute: everything allowed.
pub fn gate(mode: Mode, effect: ToolEffect) -> ToolGate {
    match mode {
        Mode::Plan => match effect {
            ToolEffect::ReadOnly => ToolGate::Allow,
            _ => ToolGate::Deny,
        },
        Mode::Code => match effect {
            ToolEffect::ReadOnly | ToolEffect::WorkspaceWrite => ToolGate::Allow,
            ToolEffect::ProcessExecution => ToolGate::RequireApproval,
        },
        Mode::Confirm => ToolGate::RequireApproval,
        Mode::Auto | Mode::Execute => ToolGate::Allow,
    }
}

/// Mode gate that also consults the command risk classifier. Only Code mode
/// gains the exemption: commands not flagged dangerous run without asking
/// (ZCode 自主模式); Plan still denies every side effect, Confirm still asks
/// for everything, Auto already allows everything.
pub fn gate_call(mode: Mode, effect: ToolEffect, command: Option<&str>) -> ToolGate {
    if mode == Mode::Code
        && effect == ToolEffect::ProcessExecution
        && command.is_some_and(|cmd| !command_policy::is_dangerous(cmd))
    {
        return ToolGate::Allow;
    }
    gate(mode, effect)
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
        assert_eq!(gate(Mode::Code, ToolEffect::WorkspaceWrite), Allow);
        assert_eq!(gate(Mode::Code, ToolEffect::ProcessExecution), RequireApproval);
    }

    #[test]
    fn code_mode_gate_requires_approval_for_commands() {
        assert_eq!(authorize(Mode::Code, "write_file").unwrap(), ToolEffect::WorkspaceWrite);
        assert_eq!(authorize(Mode::Code, "read_file").unwrap(), ToolEffect::ReadOnly);
        assert_eq!(gate(Mode::Code, ToolEffect::WorkspaceWrite), ToolGate::Allow);
        assert_eq!(gate(Mode::Code, ToolEffect::ProcessExecution), ToolGate::RequireApproval);
        for tool in ["run_command", "mcp__fs__write_file"] {
            assert_eq!(authorize(Mode::Code, tool).unwrap(), ToolEffect::ProcessExecution);
        }
    }

    #[test]
    fn code_mode_asks_only_for_dangerous_commands() {
        use ToolGate::*;
        // 自主模式：非危险命令直接放行，危险命令才挂起。
        for command in [
            "git status",
            "rg TODO",
            "where cmd",
            "npm test",
            "npm install",
            "python run_tests.py",
            "pnpm build",
            "cargo test",
            "git add .",
            "git commit -m x",
            "dir",
            "git diff --output=x",
        ] {
            assert_eq!(
                gate_call(Mode::Code, ToolEffect::ProcessExecution, Some(command)),
                Allow,
                "{command}"
            );
        }
        for command in [
            "git push",
            "git checkout .",
            "npm publish",
            "del x",
            "rm -rf x",
            "python -c \"print(1)\"",
            "git status && del x",
            "cargo publish",
            "docker ps",
            "curl https://x",
            "cmd /C dir",
        ] {
            assert_eq!(
                gate_call(Mode::Code, ToolEffect::ProcessExecution, Some(command)),
                RequireApproval,
                "{command}"
            );
        }
        assert_eq!(gate_call(Mode::Code, ToolEffect::ProcessExecution, None), RequireApproval);
        // The exemption is Code-only.
        assert_eq!(gate_call(Mode::Confirm, ToolEffect::ProcessExecution, Some("git status")), RequireApproval);
        assert_eq!(gate_call(Mode::Plan, ToolEffect::ProcessExecution, Some("git status")), Deny);
        assert_eq!(gate_call(Mode::Auto, ToolEffect::ProcessExecution, Some("git push")), Allow);
        // Non-command effects keep their existing matrix.
        assert_eq!(gate_call(Mode::Code, ToolEffect::WorkspaceWrite, Some("git status")), Allow);
        assert_eq!(gate_call(Mode::Plan, ToolEffect::WorkspaceWrite, Some("git status")), Deny);
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
