//! Process execution adapter: the `run_command` tool body. Commands run
//! through the platform shell inside the workspace root with a hard timeout,
//! a bounded output budget and a killable process tree — the model never
//! controls cwd, lifetime, or limits.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
};

use crate::domain::AppResult;

pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_COMMAND_CHARS: usize = 4096;
/// Per-stream retention cap. The pipes keep being drained past this point so a
/// chatty child can never block, but the retained bytes (and therefore memory)
/// stay bounded regardless of how much the command prints.
const MAX_STREAM_BYTES: usize = 65_536;
const MAX_OUTPUT_CHARS: usize = 65_536;
/// How long a killed tree gets to close its pipes before the drain tasks are
/// abandoned. A grandchild that escaped the job can still hold them open.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Execute `command` via the platform shell (`cmd /C` on Windows, `sh -c`
/// elsewhere) with cwd pinned to the workspace root. Used for commands that
/// were not allowlisted, i.e. after the user approved them.
pub async fn run_command(root: &Path, command: &str) -> AppResult<String> {
    run_command_with_cancel(root, command, COMMAND_TIMEOUT, None).await
}

pub async fn run_command_with_timeout(
    root: &Path,
    command: &str,
    timeout: Duration,
) -> AppResult<String> {
    run_command_with_cancel(root, command, timeout, None).await
}

/// Same as [`run_command`], but the caller may cancel the command mid-flight.
/// On timeout or cancellation the whole process tree is terminated, the pipes
/// are drained for at most [`DRAIN_GRACE`], then the drain tasks are abandoned.
pub async fn run_command_with_cancel(
    root: &Path,
    command: &str,
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
) -> AppResult<String> {
    let command = command.trim();
    if command.is_empty() || command.len() > MAX_COMMAND_CHARS {
        return Err("命令为空或超过 4096 字符限制".into());
    }
    let cwd = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let process = if cfg!(windows) {
        let mut process = Command::new("cmd");
        process.args(["/C", command]);
        process
    } else {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    spawn_and_collect(process, &cwd, timeout, cancel, "无法启动 shell 进程").await
}

/// Run an already-allowlisted program directly — no shell, so no quoting,
/// expansion or compound-command semantics apply. `program` must be an
/// absolute path resolved through [`resolve_program`].
pub async fn run_program(
    root: &Path,
    program: &Path,
    args: &[String],
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
) -> AppResult<String> {
    let cwd = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let mut process = Command::new(program);
    process.args(args);
    apply_git_env(&mut process, program);
    spawn_and_collect(process, &cwd, timeout, cancel, "无法启动命令").await
}

/// git would otherwise spawn a pager and refresh the index; both would make
/// a "read-only" command write or block.
fn apply_git_env(process: &mut Command, program: &Path) {
    if program
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("git"))
    {
        process
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0");
    }
}

/// Execute an allowlisted read-only pipeline: direct argv processes chained
/// stdout→stdin through real pipe fds — never a shell anywhere. Large upstream
/// output streams through instead of hitting memory caps, and a `head`-style
/// terminator backpressures the chain exactly like a shell pipeline.
pub async fn run_pipeline(
    root: &Path,
    segments: &[(PathBuf, Vec<String>)],
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
) -> AppResult<String> {
    let cwd = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    if segments.is_empty() {
        return Err("空管道".into());
    }
    let mut children: Vec<Child> = Vec::with_capacity(segments.len());
    let mut trees = Vec::new();
    let mut stderr_tasks = Vec::new();
    let mut prev_stdout: Option<tokio::process::ChildStdout> = None;
    let mut names: Vec<String> = Vec::new();
    for (program, args) in segments {
        let mut process = Command::new(program);
        process.args(args);
        apply_git_env(&mut process, program);
        names.push(format!(
            "{} {}",
            program.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            args.join(" ")
        ));
        let stdin = match prev_stdout.take() {
            Some(stdout) => {
                let stdout: std::process::Stdio = stdout.try_into().map_err(|_| "无法接管管道输出".to_string())?;
                stdout
            }
            None => std::process::Stdio::null(),
        };
        let mut child = process
            .current_dir(&cwd)
            .stdin(stdin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| format!("无法启动命令：{}", program.display()))?;
        prev_stdout = child.stdout.take();
        if let Some(stderr) = child.stderr.take() {
            stderr_tasks.push(tokio::spawn(drain_bounded(stderr)));
        }
        trees.push(ProcessTree::attach(&child));
        children.push(child);
    }
    let stdout_pipe = prev_stdout.ok_or("无法获取 stdout")?;
    let stdout_task = tokio::spawn(drain_bounded(stdout_pipe));

    // 等全部子进程退出；timeout/cancel 时整链回收。select 取消等待 future 不
    // 会 drop Child（在 vec 里），必须显式逐段 terminate。
    let wait_all = async {
        let mut codes = Vec::new();
        for child in children.iter_mut() {
            codes.push(child.wait().await);
        }
        codes
    };
    let mut cancel = cancel;
    enum PipeEnd {
        Exited(Vec<std::io::Result<std::process::ExitStatus>>),
        TimedOut,
        Cancelled,
    }
    let end = tokio::select! {
        codes = wait_all => PipeEnd::Exited(codes),
        () = tokio::time::sleep(timeout) => PipeEnd::TimedOut,
        () = wait_for_cancel(cancel.as_mut()) => PipeEnd::Cancelled,
    };
    if !matches!(end, PipeEnd::Exited(_)) {
        for (tree, child) in trees.iter_mut().zip(children.iter_mut()) {
            tree.terminate(child);
            let _ = child.wait().await;
        }
        let _ = join_drain(stdout_task).await;
        for task in stderr_tasks {
            let _ = join_drain(task).await;
        }
        return Err(match end {
            PipeEnd::TimedOut => format!("命令执行超时（{}s），已终止", timeout.as_secs()),
            _ => "命令已取消".into(),
        });
    }
    let PipeEnd::Exited(results) = end else { unreachable!() };
    let (stdout, stdout_dropped) = join_drain(stdout_task).await;
    let mut stderr_all: Vec<u8> = Vec::new();
    let mut stderr_dropped = 0u64;
    for task in stderr_tasks {
        let (bytes, dropped) = join_drain(task).await;
        stderr_all.extend_from_slice(&bytes);
        stderr_dropped += dropped;
    }
    let mut failures = Vec::new();
    for (index, status) in results.iter().enumerate() {
        let code = status.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);
        if code != 0 {
            failures.push(format!("第 {} 段（{}）退出码 {}", index + 1, names[index], code));
        }
    }
    let last_code = results
        .last()
        .and_then(|s| s.as_ref().ok())
        .and_then(|s| s.code())
        .unwrap_or(-1);
    let mut text = format!("exit code {last_code}");
    if !failures.is_empty() {
        text.push_str(&format!("（{}）", failures.join("；")));
    }
    append_stream(&mut text, "stdout", &stdout, stdout_dropped);
    append_stream(&mut text, "stderr", &stderr_all, stderr_dropped);
    Ok(text)
}

/// Resolve a bare program name through `PATH`, never through the current
/// directory and never to a file inside the workspace: a repository that drops
/// a `git.exe` next to its sources must not be able to hijack the allowlist.
pub fn resolve_program(name: &str, path: &OsStr, workspace_root: &Path) -> Option<PathBuf> {
    let root = workspace_root.canonicalize().ok()?;
    for dir in std::env::split_paths(path) {
        if dir.as_os_str().is_empty() {
            // An empty PATH entry means "current directory" on Windows.
            continue;
        }
        for candidate_name in program_candidates(name) {
            let candidate = dir.join(candidate_name);
            if !candidate.is_file() {
                continue;
            }
            let Ok(resolved) = candidate.canonicalize() else { continue };
            if resolved.starts_with(&root) {
                continue;
            }
            return Some(resolved);
        }
    }
    None
}

/// Convenience wrapper that reads the process `PATH`.
pub fn resolve_in_path(name: &str, workspace_root: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    resolve_program(name, &path, workspace_root)
}

fn program_candidates(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
}

async fn spawn_and_collect(
    mut process: Command,
    cwd: &Path,
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
    spawn_error: &str,
) -> AppResult<String> {
    let mut child = process
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| spawn_error.to_string())?;
    let stdout_pipe = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr_pipe = child.stderr.take().ok_or("无法获取 stderr")?;
    let mut tree = ProcessTree::attach(&child);
    // Drain concurrently so a chatty child cannot deadlock on a full pipe
    // buffer, while retaining only the first MAX_STREAM_BYTES of each stream.
    let stdout_task = tokio::spawn(drain_bounded(stdout_pipe));
    let stderr_task = tokio::spawn(drain_bounded(stderr_pipe));

    let mut cancel = cancel;
    let end = tokio::select! {
        status = child.wait() => End::Exited(status),
        () = tokio::time::sleep(timeout) => End::TimedOut,
        () = wait_for_cancel(cancel.as_mut()) => End::Cancelled,
    };

    match end {
        End::Exited(Ok(status)) => {
            let code = status.code().unwrap_or(-1);
            let (stdout, stdout_dropped) = join_drain(stdout_task).await;
            let (stderr, stderr_dropped) = join_drain(stderr_task).await;
            let mut text = format!("exit code {code}");
            append_stream(&mut text, "stdout", &stdout, stdout_dropped);
            append_stream(&mut text, "stderr", &stderr, stderr_dropped);
            Ok(text)
        }
        End::Exited(Err(_)) => {
            tree.terminate(&mut child);
            let _ = child.wait().await;
            Err("无法读取命令输出".into())
        }
        End::TimedOut => {
            tree.terminate(&mut child);
            let _ = child.wait().await;
            let _ = join_drain(stdout_task).await;
            let _ = join_drain(stderr_task).await;
            Err(format!("命令执行超时（{}s），已终止", timeout.as_secs()))
        }
        End::Cancelled => {
            tree.terminate(&mut child);
            let _ = child.wait().await;
            let _ = join_drain(stdout_task).await;
            let _ = join_drain(stderr_task).await;
            Err("命令已取消".into())
        }
    }
}

enum End {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
    Cancelled,
}

/// A detached command: pipes drain into the shared bounded buffer, the
/// waiter task applies the same timeout/cancel/tree-kill semantics as the
/// foreground path, and `status` transitions to a terminal label exactly
/// once (`exited:<code>` / `timeout` / `cancelled` / `failed`).
pub struct BackgroundHandle {
    pub output: std::sync::Arc<std::sync::Mutex<String>>,
    pub status: std::sync::Arc<std::sync::Mutex<String>>,
    pub cancel: watch::Sender<bool>,
}

/// Spawn `command` without awaiting it (ZCode `run_in_background`): returns a
/// handle for `task_output`/`task_stop`. The process tree is still bound to a
/// job object and killed on timeout/cancel — nothing escapes the sandbox.
pub fn spawn_background(
    root: &Path,
    command: &str,
    timeout: Duration,
) -> AppResult<BackgroundHandle> {
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
    let stdout_pipe = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr_pipe = child.stderr.take().ok_or("无法获取 stderr")?;
    let mut tree = ProcessTree::attach(&child);
    let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let status = std::sync::Arc::new(std::sync::Mutex::new("running".to_string()));
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let sink = output.clone();
    tokio::spawn(drain_labelled(stdout_pipe, "stdout", sink));
    let sink = output.clone();
    tokio::spawn(drain_labelled(stderr_pipe, "stderr", sink));
    let status_slot = status.clone();
    tokio::spawn(async move {
        let end = tokio::select! {
            result = child.wait() => End::Exited(result),
            () = tokio::time::sleep(timeout) => End::TimedOut,
            () = wait_for_cancel(Some(&mut cancel_rx)) => End::Cancelled,
        };
        let label = match end {
            End::Exited(Ok(status)) => {
                format!("exited:{}", status.code().unwrap_or(-1))
            }
            End::Exited(Err(_)) => "failed".to_string(),
            End::TimedOut => "timeout".to_string(),
            End::Cancelled => "cancelled".to_string(),
        };
        if !matches!(end, End::Exited(_)) {
            tree.terminate(&mut child);
            let _ = child.wait().await;
        }
        *status_slot.lock().unwrap() = label;
    });
    Ok(BackgroundHandle { output, status, cancel: cancel_tx })
}

/// Drain one stream into the shared buffer with a header once it produces
/// content; retention stays bounded so a chatty server cannot grow memory.
async fn drain_labelled<R>(mut pipe: R, label: &str, out: std::sync::Arc<std::sync::Mutex<String>>)
where
    R: AsyncReadExt + Unpin,
{
    let mut header_written = false;
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let chunk = String::from_utf8_lossy(&buf[..read]);
                let mut guard = out.lock().unwrap();
                let room = MAX_OUTPUT_CHARS.saturating_sub(guard.len());
                if room == 0 {
                    continue;
                }
                if !header_written {
                    guard.push_str(&format!("--- {label} ---\n"));
                    header_written = true;
                }
                let take = chunk.chars().take(room).collect::<String>();
                guard.push_str(&take);
            }
        }
    }
}

async fn wait_for_cancel(cancel: Option<&mut watch::Receiver<bool>>) {
    let Some(receiver) = cancel else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            // Sender gone without a cancel request: never resolve.
            std::future::pending::<()>().await;
        }
    }
}

async fn drain_bounded<R>(mut pipe: R) -> (Vec<u8>, u64)
where
    R: AsyncReadExt + Unpin,
{
    let mut kept = Vec::new();
    let mut dropped = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let room = MAX_STREAM_BYTES.saturating_sub(kept.len());
                let take = read.min(room);
                kept.extend_from_slice(&buf[..take]);
                dropped += (read - take) as u64;
            }
        }
    }
    (kept, dropped)
}

async fn join_drain(mut task: JoinHandle<(Vec<u8>, u64)>) -> (Vec<u8>, u64) {
    tokio::select! {
        result = &mut task => result.unwrap_or((Vec::new(), 0)),
        () = tokio::time::sleep(DRAIN_GRACE) => {
            task.abort();
            (Vec::new(), 0)
        }
    }
}

fn append_stream(text: &mut String, label: &str, bytes: &[u8], dropped: u64) {
    let stream = String::from_utf8_lossy(bytes);
    if stream.trim().is_empty() && dropped == 0 {
        return;
    }
    text.push_str(&format!(
        "\n--- {label} ---\n{}",
        stream.chars().take(MAX_OUTPUT_CHARS).collect::<String>()
    ));
    if dropped > 0 {
        text.push_str(&format!("\n…（输出已截断：省略 {dropped} 字节）"));
    }
}

/// Windows job object bound to the spawned shell. Closing the last handle
/// terminates the whole tree, which is what `kill_on_drop` alone cannot do for
/// grandchildren. Attaching is best-effort: if any step fails we keep the
/// single-process kill and report the limitation through the same behaviour.
struct ProcessTree {
    #[cfg(windows)]
    job: Option<JobHandle>,
}

#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// The job handle is only ever used from the task that owns the child; handing
// the raw pointer across threads is what tokio's spawn bound needs.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

impl ProcessTree {
    #[cfg(windows)]
    fn attach(child: &tokio::process::Child) -> Self {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JobObjectExtendedLimitInformation,
                },
                Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
            },
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Self { job: None };
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            let Some(pid) = child.id() else {
                CloseHandle(job);
                return Self { job: None };
            };
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if configured == 0 || process.is_null() {
                CloseHandle(job);
                if !process.is_null() {
                    CloseHandle(process);
                }
                return Self { job: None };
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                CloseHandle(job);
                return Self { job: None };
            }
            Self { job: Some(JobHandle(job)) }
        }
    }

    #[cfg(not(windows))]
    fn attach(_child: &tokio::process::Child) -> Self {
        Self {}
    }

    fn terminate(&mut self, child: &mut tokio::process::Child) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            if let Some(job) = &self.job {
                unsafe {
                    TerminateJobObject(job.0, 1);
                }
                return;
            }
        }
        let _ = child.start_kill();
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(job.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn resolve_program_skips_the_workspace_and_empty_path_entries() {
        let base = tempfile::tempdir().unwrap();
        let bin = base.path().join("bin");
        let workspace = base.path().join("workspace");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let tool = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(bin.join(tool), b"x").unwrap();
        std::fs::write(workspace.join(tool), b"x").unwrap();

        // A workspace-local binary must never win, even when it is first in PATH.
        let path = std::env::join_paths([workspace.as_os_str(), bin.as_os_str()]).unwrap();
        assert_eq!(
            resolve_program("tool", &path, &workspace).unwrap(),
            bin.join(tool).canonicalize().unwrap()
        );

        let only_workspace = std::env::join_paths([workspace.as_os_str()]).unwrap();
        assert!(resolve_program("tool", &only_workspace, &workspace).is_none());

        // An empty PATH entry means "current directory" on Windows: skipped.
        let with_empty = std::env::join_paths([OsStr::new(""), bin.as_os_str()]).unwrap();
        assert!(resolve_program("tool", &with_empty, &workspace).is_some());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn run_program_runs_an_allowlisted_query_without_a_shell() {
        let root = tempfile::tempdir().unwrap();
        let program = resolve_in_path("where", root.path()).expect("where.exe must exist");
        let out = run_program(
            root.path(),
            &program,
            &["cmd".to_string()],
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(out.contains("exit code 0"), "{out}");
        assert!(out.to_lowercase().contains("cmd.exe"), "{out}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_program_passes_metacharacters_literally() {
        let root = tempfile::tempdir().unwrap();
        let out = run_program(
            root.path(),
            Path::new("/bin/echo"),
            &["a&b".to_string(), "c".to_string()],
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(out.contains("a&b c"), "{out}");
    }

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

    #[tokio::test]
    async fn drain_bounded_keeps_a_cap_and_counts_the_rest() {
        let data = vec![b'x'; MAX_STREAM_BYTES + 5_000];
        let (kept, dropped) = drain_bounded(&data[..]).await;
        assert_eq!(kept.len(), MAX_STREAM_BYTES);
        assert_eq!(dropped, 5_000);
    }

    #[tokio::test]
    async fn output_beyond_the_cap_is_truncated_and_reported() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("big.txt"), "x".repeat(200_000)).unwrap();
        let command = if cfg!(windows) { "type big.txt" } else { "cat big.txt" };
        let out = run_command_with_timeout(root.path(), command, Duration::from_secs(30))
            .await
            .unwrap();
        assert!(out.contains("输出已截断"), "{}", &out[..out.len().min(200)]);
        assert!(out.len() <= MAX_OUTPUT_CHARS + 512, "retained {} chars", out.len());
    }

    #[tokio::test]
    async fn a_cancel_request_ends_the_command_immediately() {
        let root = tempfile::tempdir().unwrap();
        let (cancel, receiver) = watch::channel(false);
        let task = tokio::spawn({
            let root = root.path().to_path_buf();
            async move {
                run_command_with_cancel(
                    &root,
                    if cfg!(windows) { "ping -n 30 127.0.0.1" } else { "sleep 30" },
                    Duration::from_secs(60),
                    Some(receiver),
                )
                .await
            }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.send(true).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel must end the command, not wait for the 60s timeout")
            .unwrap();
        assert!(result.unwrap_err().contains("取消"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_chains_stdout_into_stdin_without_a_shell() {
        let root = tempfile::tempdir().unwrap();
        let echo = resolve_in_path("echo", root.path()).expect("echo must exist");
        let sort = resolve_in_path("sort", root.path()).expect("sort must exist");
        let head = resolve_in_path("head", root.path()).expect("head must exist");
        let out = run_pipeline(
            root.path(),
            &[
                (echo, ["b
a
c".to_string()].into()),
                (sort, vec![]),
                (head, ["-n".into(), "2".into()]),
            ],
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(out.contains("exit code 0"), "{out}");
        // sort 后 head 截两行：a、b——顺序证明 stdout 真的接进了 stdin。
        assert!(out.contains("a"), "{out}");
        assert!(out.contains("b"), "{out}");
        assert!(!out.contains("
c"), "{out}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_reports_segment_failure() {
        let root = tempfile::tempdir().unwrap();
        let echo = resolve_in_path("echo", root.path()).expect("echo must exist");
        let grep = resolve_in_path("grep", root.path()).expect("grep must exist");
        // grep 无匹配退出码 1——如实上报段失败而不是静默。
        let out = run_pipeline(
            root.path(),
            &[
                (echo, ["hello".to_string()].into()),
                (grep, ["zzz-no-match".to_string()].into()),
            ],
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(out.contains("退出码 1"), "{out}");
    }
}
