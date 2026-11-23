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
/// - Plan: read-only only. Non-read-only commands/MCP stay `Deny` — only
///   allowlisted read-only commands earn an approval path via `gate_call`.
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

/// Whether the tool spec should be advertised to the model in `mode`. Denied
/// tools are stripped — except Plan still advertises `run_command`: the spec
/// cannot be argument-aware, and allowlisted read-only commands are callable
/// there (each call suspends for approval). Everything else stays hidden.
pub fn spec_visible(mode: Mode, tool: &str, effect: ToolEffect) -> bool {
    match gate(mode, effect) {
        ToolGate::Deny => {
            mode == Mode::Plan && tool == "run_command" && effect == ToolEffect::ProcessExecution
        }
        _ => true,
    }
}

/// Mode gate that also consults the command policy. `tool` disambiguates the
/// `command` argument — MCP tools may carry their own `command` field.
/// - Plan: only allowlisted read-only commands (`is_read_only_safe`, no
///   `background`) suspend for approval; every other side effect stays Deny.
/// - Code: commands not flagged dangerous run without asking (ZCode 自主模式).
/// - Confirm still asks for everything, Auto already allows everything.
pub fn gate_call(mode: Mode, tool: &str, effect: ToolEffect, command: Option<&str>) -> ToolGate {
    if mode == Mode::Plan
        && tool == "run_command"
        && effect == ToolEffect::ProcessExecution
        && command.is_some_and(command_policy::is_read_only_safe)
    {
        return ToolGate::RequireApproval;
    }
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
        // authorize 没有参数上下文：Plan 下副作用调用一律按 Deny 处理；
        // 白名单只读命令的审批通道在 gate_call 一层按命令文本放行。
        for tool in ["apply_patch", "write_file", "run_command"] {
            assert!(authorize(Mode::Plan, tool).is_err());
        }
    }

    #[test]
    fn plan_mode_asks_only_for_allowlisted_read_only_commands() {
        use ToolGate::*;
        // 白名单只读命令：挂起等批准，批准后走无 shell exec。
        for command in ["git status", "rg TODO", "cat a.txt", "ls", "node -v", "where cargo"] {
            assert_eq!(
                gate_call(Mode::Plan, "run_command", ToolEffect::ProcessExecution, Some(command)),
                RequireApproval,
                "{command}"
            );
        }
        // 非白名单命令（危险/写/任意 shell 组合）在 Plan 下仍是硬拒。
        for command in ["npm install", "git push", "del x", "cargo test", "git status && del x", "unknown_tool"] {
            assert_eq!(
                gate_call(Mode::Plan, "run_command", ToolEffect::ProcessExecution, Some(command)),
                Deny,
                "{command}"
            );
        }
        assert_eq!(gate_call(Mode::Plan, "run_command", ToolEffect::ProcessExecution, None), Deny);
        // MCP 工具也可能自带 `command` 参数——按 tool 名消歧，不误判成可审批命令。
        assert_eq!(
            gate_call(Mode::Plan, "mcp__sh__run", ToolEffect::ProcessExecution, Some("git status")),
            Deny
        );
        // 写操作与 readOnly 效果不受 run_command 分支影响。
        assert_eq!(gate_call(Mode::Plan, "write_file", ToolEffect::WorkspaceWrite, Some("git status")), Deny);
        assert_eq!(gate_call(Mode::Plan, "read_file", ToolEffect::ReadOnly, None), Allow);
    }

    #[test]
    fn spec_visible_hides_denied_tools_but_keeps_plan_run_command() {
        assert!(spec_visible(Mode::Plan, "read_file", ToolEffect::ReadOnly));
        assert!(spec_visible(Mode::Plan, "run_command", ToolEffect::ProcessExecution));
        assert!(!spec_visible(Mode::Plan, "write_file", ToolEffect::WorkspaceWrite));
        assert!(!spec_visible(Mode::Plan, "apply_patch", ToolEffect::WorkspaceWrite));
        assert!(!spec_visible(Mode::Plan, "mcp__sh__run", ToolEffect::ProcessExecution));
        assert!(spec_visible(Mode::Plan, "mcp__wiki__search", ToolEffect::ReadOnly));
        // 其它模式的可见性与 gate 结果一致。
        assert!(spec_visible(Mode::Code, "run_command", ToolEffect::ProcessExecution));
        assert!(spec_visible(Mode::Confirm, "write_file", ToolEffect::WorkspaceWrite));
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
                gate_call(Mode::Code, "run_command", ToolEffect::ProcessExecution, Some(command)),
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
                gate_call(Mode::Code, "run_command", ToolEffect::ProcessExecution, Some(command)),
                RequireApproval,
                "{command}"
            );
        }
        assert_eq!(gate_call(Mode::Code, "run_command", ToolEffect::ProcessExecution, None), RequireApproval);
        // The exemption is Code-only.
        assert_eq!(gate_call(Mode::Confirm, "run_command", ToolEffect::ProcessExecution, Some("git status")), RequireApproval);
        assert_eq!(gate_call(Mode::Plan, "run_command", ToolEffect::ProcessExecution, Some("git push")), Deny);
        assert_eq!(gate_call(Mode::Auto, "run_command", ToolEffect::ProcessExecution, Some("git push")), Allow);
        // Non-command effects keep their existing matrix.
        assert_eq!(gate_call(Mode::Code, "write_file", ToolEffect::WorkspaceWrite, Some("git status")), Allow);
        assert_eq!(gate_call(Mode::Plan, "write_file", ToolEffect::WorkspaceWrite, Some("git status")), Deny);
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
