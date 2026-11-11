//! Process execution adapter: the `run_command` tool body. Commands run
//! through the platform shell inside the workspace root with a hard timeout
//! and bounded output — the model never controls cwd, lifetime, or limits.
use std::{path::Path, time::Duration};

use tokio::process::Command;

use crate::domain::AppResult;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_COMMAND_CHARS: usize = 4096;
const MAX_OUTPUT_CHARS: usize = 65536;

/// Execute `command` via the platform shell (`cmd /C` on Windows, `sh -c`
/// elsewhere) with cwd pinned to the workspace root. On timeout the child is
/// killed (`kill_on_drop`); a non-zero exit code still returns text — the
/// reported `exit code N` is a mechanical fact the model cannot spin into a
/// success claim. Grandchildren of the shell may outlive a kill (documented
/// limitation; a full tree kill would need a platform job object).
pub async fn run_command(root: &Path, command: &str) -> AppResult<String> {
    run_command_with_timeout(root, command, COMMAND_TIMEOUT).await
}

pub async fn run_command_with_timeout(
    root: &Path,
    command: &str,
    timeout: Duration,
) -> AppResult<String> {
    let command = command.trim();
    if command.is_empty() || command.len() > MAX_COMMAND_CHARS {
        return Err("命令为空或超过 4096 字符限制".into());
    }
    let cwd = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let mut process = if cfg!(windows) {
        let mut process = Command::new("cmd");
        process.args(["/C", command]);
        process
    } else {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    let mut child = process
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "无法启动 shell 进程".to_string())?;
    let mut stdout_pipe = child.stdout.take().ok_or("无法获取 stdout")?;
    let mut stderr_pipe = child.stderr.take().ok_or("无法获取 stderr")?;
    // Drain the pipes concurrently so a chatty child cannot deadlock on a
    // full pipe buffer while we wait on it.
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stdout_pipe, &mut buf)
            .await
            .map(|_| buf)
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stderr_pipe, &mut buf)
            .await
            .map(|_| buf)
    });
    let status = tokio::select! {
        status = child.wait() => Some(status),
        () = tokio::time::sleep(timeout) => None,
    };
    match status {
        Some(Ok(status)) => {
            let code = status.code().unwrap_or(-1);
            let stdout = stdout_task.await.ok().and_then(|r| r.ok()).unwrap_or_default();
            let stderr = stderr_task.await.ok().and_then(|r| r.ok()).unwrap_or_default();
            let stdout = String::from_utf8_lossy(&stdout);
            let stderr = String::from_utf8_lossy(&stderr);
            let mut text = format!("exit code {code}");
            if !stdout.trim().is_empty() {
                text.push_str(&format!(
                    "\n--- stdout ---\n{}",
                    stdout.chars().take(MAX_OUTPUT_CHARS).collect::<String>()
                ));
            }
            if !stderr.trim().is_empty() {
                text.push_str(&format!(
                    "\n--- stderr ---\n{}",
                    stderr.chars().take(MAX_OUTPUT_CHARS).collect::<String>()
                ));
            }
            Ok(text)
        }
        Some(Err(_)) => Err("无法读取命令输出".into()),
        None => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(format!("命令执行超时（{}s），已终止", timeout.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_runs_inside_the_workspace_and_reports_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let out = run_command(root.path(), "echo hello-maestro").await.unwrap();
        assert!(out.contains("exit code 0"), "{out}");
        assert!(out.contains("hello-maestro"), "{out}");
    }

    #[tokio::test]
    async fn nonzero_exit_is_reported_as_fact_not_success() {
        let root = tempfile::tempdir().unwrap();
        let out = run_command(root.path(), "exit 3").await.unwrap();
        assert!(out.contains("exit code 3"), "{out}");
    }

    #[tokio::test]
    async fn empty_or_oversized_commands_are_refused() {
        let root = tempfile::tempdir().unwrap();
        assert!(run_command(root.path(), "   ").await.is_err());
        assert!(run_command(root.path(), &"x".repeat(4097)).await.is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn timeout_kills_the_child() {
        let root = tempfile::tempdir().unwrap();
        // ~9s of pings must die at the 500ms timeout.
        let result =
            run_command_with_timeout(root.path(), "ping -n 10 127.0.0.1", Duration::from_millis(500))
                .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("超时"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_child() {
        let root = tempfile::tempdir().unwrap();
        let result =
            run_command_with_timeout(root.path(), "sleep 10", Duration::from_millis(500)).await;
        assert!(result.is_err());
    }
}
