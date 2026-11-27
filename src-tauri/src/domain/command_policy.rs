//! Command classification for Code mode (ZCode 「自主模式」/ guarded semantics):
//! commands run without asking unless a danger rule hits.
//!
//! Three tiers:
//! - [`CommandPolicy::ReadOnly`]: a strict **allow** list — program, subcommand
//!   and every flag must be listed. These execute **without a shell** as a
//!   parsed argv, so quoting/expansion never happens.
//! - [`CommandPolicy::Auto`]: nothing dangerous matched — run through the
//!   platform shell without prompting (`python run_tests.py`, `npm test`…).
//! - [`CommandPolicy::Dangerous`]: a segment matched the risk classifier —
//!   suspend for approval.
//!
//! The risk classifier is **best-effort**, not a security boundary: a blacklist
//! cannot bound `cmd /C` (indirection, argument variants), so nested executors
//! (shells, `-c`/`-e` inline code, `xargs`, `for`…) always ask — that is the
//! largest bounded backstop. Everything else errs toward the user's habit of
//! rarely being interrupted.
//!
//! The read-only parser is still conservative about metacharacters: they are
//! rejected outright so a future change that reintroduces a shell cannot turn
//! an accepted command into a compound one.

/// Same ceiling as the exec adapter: longer strings are never parsed.
const MAX_COMMAND_CHARS: usize = 4096;

/// Characters that only mean something to a shell. Rejected anywhere in the
/// command, including inside quotes, so an accepted command is always a single
/// literal invocation.
const SHELL_METACHARS: &[char] = &[
    '&', '|', ';', '<', '>', '^', '%', '$', '`', '(', ')', '{', '}', '[', ']', '*', '?', '~',
    '!', '#',
];

/// One allowlisted argv in a pipeline — program name plus literal arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlySegment {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPolicy {
    /// Parsed and allowlisted: run directly (no shell). A single command is a
    /// one-segment pipeline; multi-segment pipelines wire stdout→stdin manually
    /// — still no shell anywhere, so quoting/expansion never happens.
    ReadOnly { segments: Vec<ReadOnlySegment> },
    /// 自主放行：没有命中危险规则——走平台 shell 执行，无需审批。
    Auto,
    /// 命中危险规则：挂起等待审批，批准后走平台 shell 执行。
    Dangerous,
}

/// Classify one `run_command` string.
pub fn parse(command: &str) -> CommandPolicy {
    if command.len() > MAX_COMMAND_CHARS {
        return CommandPolicy::Dangerous;
    }
    if let Some(segments) = read_only_pipeline(command) {
        return CommandPolicy::ReadOnly { segments };
    }
    if is_dangerous(command) {
        CommandPolicy::Dangerous
    } else {
        CommandPolicy::Auto
    }
}

/// 管道段数上限：`git ls-tree | sort | head` 三段是真实工作形态，更多段
/// 往往是模型在拼凑 shell 脚本——超出即回落危险分类（Code 审批/Plan 拒绝）。
const MAX_PIPE_SEGMENTS: usize = 4;

/// Split on **unquoted** `|` only. Every other shell metachar stays fatal
/// inside `tokenize`, so this is the one place pipes become structure.
/// Quoted pipes never split — but note `tokenize` still rejects metachars
/// inside quotes, so they are not a smuggling vector either way.
fn split_pipes(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        match quote {
            Some(open) => {
                current.push(ch);
                if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                '|' => out.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            },
        }
    }
    out.push(current);
    out
}

/// Classify one tokenized argv against the allowlist; returns the resolved
/// program and args when every element is listed.
fn classify_segment(tokens: &[String]) -> Option<(String, Vec<String>)> {
    let program = program_name(tokens.first()?)?;
    let args = &tokens[1..];
    let safe = match program.as_str() {
        "git" => git_args(args),
        "rg" => args_match(args, RG_FLAGS, true) && positionals_are_relative(args),
        "fd" => args_match(args, FD_FLAGS, true) && positionals_are_relative(args),
        "grep" => args_match(args, GREP_FLAGS, true) && positionals_are_relative(args),
        "cat" => args_match(args, CAT_FLAGS, true) && positionals_are_relative(args),
        "head" | "tail" => {
            args_match(args, HEAD_TAIL_FLAGS, true) && positionals_are_relative(args)
        }
        "wc" => args_match(args, WC_FLAGS, true) && positionals_are_relative(args),
        "sort" => {
            let expanded = expand_short_clusters(args, "nrubfsVMhz", "kt");
            args_match(&expanded, SORT_FLAGS, true) && positionals_are_relative(&expanded)
        }
        "more" => {
            positionals_are_relative(args) && args.iter().all(|arg| !arg.starts_with('-'))
        }
        "ls" => args_match(args, LS_FLAGS, true) && positionals_are_relative(args),
        "tree" => args_match(args, TREE_FLAGS, true) && positionals_are_relative(args),
        "pwd" | "ver" | "hostname" | "whoami" => args.is_empty(),
        "echo" => true,
        "which" | "where" => where_args(args),
        // Version probes only.
        "node" | "npm" | "python" | "python3" | "pip" | "pip3" | "cargo" | "rustc"
        | "go" | "dotnet" | "java" | "javac" | "php" | "ruby" | "deno" | "bun"
        | "pnpm" | "yarn" | "cmake" | "tsc" | "sqlite3" => {
            matches!(args, [arg] if matches!(arg.as_str(), "-v" | "-V" | "--version" | "version"))
        }
        _ => false,
    };
    safe.then(|| (program, args.to_vec()))
}

/// `-k4` / `-nr` 这类短 flag 簇在真实命令里随处可见。展开为独立 token 再
/// 过白名单：簇内字符已在 tokenize 阶段排除元字符，展开不产生新语义。
/// 仅当簇内字符全部属于已知布尔集、或首字符是取值 flag 时展开——否则原样
/// 保留（白名单拒收，失败安全）。
fn expand_short_clusters(args: &[String], bools: &str, valued: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        let bytes: Vec<char> = arg.chars().collect();
        let expandable = bytes.len() > 2 && bytes[0] == '-' && bytes[1] != '-';
        if expandable {
            let cluster: Vec<char> = bytes[1..].to_vec();
            if cluster.iter().all(|c| bools.contains(*c)) {
                for c in cluster {
                    out.push(format!("-{c}"));
                }
                continue;
            }
            if valued.contains(cluster[0]) && cluster.len() > 1 {
                out.push(format!("-{}", cluster[0]));
                out.push(cluster[1..].iter().collect());
                continue;
            }
        }
        out.push(arg.clone());
    }
    out
}

/// The whole command parses as a pipeline of allowlisted read-only segments:
/// split on unquoted pipes, then every segment must independently satisfy the
/// same strict argv allowlist as a single command.
fn read_only_pipeline(command: &str) -> Option<Vec<ReadOnlySegment>> {
    let parts = split_pipes(command);
    if parts.len() > MAX_PIPE_SEGMENTS {
        return None;
    }
    let mut segments = Vec::with_capacity(parts.len());
    for part in parts {
        let tokens = tokenize(part.trim())?;
        let (program, args) = classify_segment(&tokens)?;
        segments.push(ReadOnlySegment { program, args });
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

/// Read-only allowlist check — also selects the no-shell exec path and the
/// audit label. Mode gating uses [`is_dangerous`] instead.
pub fn is_read_only_safe(command: &str) -> bool {
    matches!(parse(command), CommandPolicy::ReadOnly { .. })
}

/// 自主模式（ZCode guarded）的风险分类：命令不命中危险规则即放行。
/// 对**原始命令串**做段级分析——复合命令的每一段独立判定。
pub fn is_dangerous(command: &str) -> bool {
    if command.len() > MAX_COMMAND_CHARS {
        return true;
    }
    // `$(...)` / 反引号命令替换可在任意参数里藏一整条命令。
    if command.contains("$(") || command.contains('`') {
        return true;
    }
    let mut saw_segment = false;
    for segment in segments(command) {
        saw_segment = true;
        if segment_is_dangerous(&segment) {
            return true;
        }
    }
    // 空命令（只有分隔符/空白）：无法分类 = 按有风险处理。
    !saw_segment
}

/// Split on spaces, honouring single/double quotes, and refuse anything a
/// shell could reinterpret. No escape sequences: `\"` is not special.
fn tokenize(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        if ch.is_control() {
            return None;
        }
        match quote {
            Some(open) => {
                if ch == open {
                    quote = None;
                } else if SHELL_METACHARS.contains(&ch) {
                    return None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                    started = true;
                } else if ch == ' ' {
                    if started {
                        tokens.push(std::mem::take(&mut current));
                        started = false;
                    }
                } else if SHELL_METACHARS.contains(&ch) {
                    return None;
                } else {
                    current.push(ch);
                    started = true;
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return None;
    }
    Some(tokens)
}

/// The program must be a bare name resolvable through PATH — never a path.
fn program_name(token: &str) -> Option<String> {
    if token.contains(['/', '\\', ':']) || token.contains("..") {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    let name = lower.strip_suffix(".exe").unwrap_or(&lower);
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return None;
    }
    Some(name.to_string())
}

// ====================== 自主模式风险分类 ======================
//
// 对原始命令串按未加引号的 shell 分隔符切段（`&&` `||` `;` `|` `&` `<` `>`
// `^` 换行），逐段判定。`>`/`<` 同样切开：下一段就是重定向目标，越界目标
// 会被通用路径检查拦下（`echo x > ../f`）。

fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(open) => {
                current.push(ch);
                if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                '&' | '|' | ';' | '<' | '>' | '^' | '\n' | '\r' => {
                    // 折叠成双的分隔符：`&&` `||` `>>` `2>&1` 的后半。
                    if matches!(chars.peek(), Some('&') | Some('|') | Some('>')) {
                        chars.next();
                    }
                    if !current.trim().is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// 段内分词：与 [`tokenize`] 不同，引号内的元字符按字面保留（它们只对
/// shell 有意义，shell 拿到的本就是整段原文）。引号不配对/控制字符返回
/// None —— 调用方按"无法分类 = 有风险"处理。
fn lex(segment: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for ch in segment.chars() {
        if ch.is_control() {
            return None;
        }
        match quote {
            Some(open) => {
                if ch == open {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                    started = true;
                } else if ch.is_whitespace() {
                    if started {
                        tokens.push(std::mem::take(&mut current));
                        started = false;
                    }
                } else {
                    current.push(ch);
                    started = true;
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        tokens.push(current);
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else { return false };
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 包装命令前缀：`time rm -rf x` 的真实程序是 `rm`。
const BENIGN_PREFIXES: &[&str] = &[
    "time", "nice", "nohup", "ionice", "stdbuf", "chrt", "taskset", "timeout",
    "caffeinate", "arch",
];

/// 嵌套执行器：载荷无法用程序名分类，一律询问（黑名单式分类的最大兜底）。
const NESTED_EXECUTORS: &[&str] = &[
    "cmd", "powershell", "pwsh", "sh", "bash", "zsh", "csh", "tcsh", "fish",
    "ksh", "dash", "wsl", "eval", "exec", "call", "source", "command",
    "builtin", "coproc", "for", "forfiles", "xargs", "watch", "env", "expect",
    "start", "invoke-expression", "iex", "screen", "tmux",
];

/// 内联代码载荷标志：`python -c`、`node -e`、`php -r`、`deno eval` 等。
const INTERPRETERS: &[&str] = &[
    "python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php",
    "lua", "julia", "rscript", "osascript", "tclsh", "groovy",
];

fn inline_code(program: &str, args: &[String]) -> bool {
    INTERPRETERS.contains(&program)
        && args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "-c" | "-e" | "-r" | "-E" | "eval" | "--eval" | "-eval"
                    | "-command" | "-encodedcommand" | "-enc" | "-ec"
            )
        })
}

/// 程序名命中即危险（无需看参数）。
const DANGEROUS_PROGRAMS: &[&str] = &[
    // 删除/破坏
    "rm", "del", "erase", "rd", "rmdir", "unlink", "shred", "srm", "wipe",
    "sfill", "sdelete", "eraser", "eraserd",
    // 磁盘/文件系统/引导
    "format", "mkfs", "fdisk", "sfdisk", "parted", "gdisk", "diskpart",
    "diskraid", "dd", "dcfldd", "mount", "umount", "mountvol", "subst",
    "fsutil", "label", "convert", "chkdsk", "chkntfs", "defrag", "compact",
    "cipher", "manage-bde", "diskshadow", "vssadmin", "wbadmin", "esentutl",
    "expand", "mklink", "junction",
    // 系统/服务/注册表/计划任务
    "shutdown", "reboot", "logoff", "poweroff", "halt", "init", "telinit",
    "systemctl", "service", "sc", "schtasks", "at", "crontab", "launchctl",
    "reg", "regedit", "regini", "regsvr32", "setx", "bcdedit", "bootcfg",
    "dism", "sfc", "pkgmgr", "sysprep", "reagentc", "winrm", "winrs",
    "slmgr", "installutil", "pnputil", "devcon", "dpinst", "verifier",
    "mofcomp", "nltest", "dsquery", "dsadd", "dsmod", "dsrm", "takeown",
    "icacls", "cacls", "attrib", "powercfg", "route", "ip", "ifconfig",
    "iwconfig", "nmcli", "iptables", "nft", "pfctl", "firewall-cmd", "netsh",
    "net", "net1", "rasdial", "rasphone", "wevtutil", "lodctr", "unlodctr",
    // 提权/远程执行
    "runas", "sudo", "su", "doas", "gsudo", "psexec", "psservice", "wmiexec",
    "smbexec", "atexec", "pass", // `pass` 密码库导出；`psexec` 系列见上
    // 进程终止
    "taskkill", "tskill", "pkill", "kill", "killall", "skill", "snice",
    "fuser", "wmic",
    // 网络外发/远程
    "ssh", "scp", "sftp", "ssh-copy-id", "sshfs", "mosh", "ftp", "tftp",
    "lftp", "ncftp", "telnet", "rlogin", "rsh", "rexec", "nc", "ncat",
    "netcat", "socat", "curl", "wget", "aria2c", "axel", "http", "https",
    "xh", "httpie", "rsync", "rclone", "syncthing", "restic", "borg",
    "duplicity", "s3cmd", "gsutil", "azcopy", "mc", "aws", "az", "gcloud",
    "oci", "vault", "consul", "proxychains", "torsocks", "tor", "nmap",
    "masscan", "hping3", "zmap", "tcpdump", "tshark", "dumpcap", "wireshark",
    "certutil", "bitsadmin", "gh", "glab",
    // 执行载体/LOLBins
    "mshta", "rundll32", "msiexec", "wscript", "cscript", "infdefaultinstall",
    "cmstp", "odbcconf", "msxsl", "dnx", "rcsi", "csi", "vshadow",
    // 容器/基础设施
    "docker", "docker-compose", "podman", "nerdctl", "buildah", "kubectl",
    "helm", "terraform", "ansible", "ansible-playbook", "vagrant", "packer",
    "minikube", "k3s", "k9s", "kind", "skaffold", "kompose",
    // OS 级包管理（写系统目录）
    "apt", "apt-get", "aptitude", "yum", "dnf", "pacman", "zypper", "apk",
    "brew", "choco", "winget", "scoop", "port", "snap", "flatpak", "emerge",
    "nix", "pipx",
    // 服务端数据库 CLI（sqlite3 走文件改写规则）
    "mysql", "mysqldump", "psql", "pg_dump", "mongo", "mongosh", "redis-cli",
    "sqlcmd", "sqlplus", "influx", "cqlsh", "clickhouse-client", "dolt",
    // 环境/凭据面（dump env 会把凭据带进模型上下文）
    "set", "printenv",
    // 版本库直连远端/破坏性操作（git 走子命令细化）
    "svnadmin", "svnsync",
    // 发布/上传
    "twine", "flit",
];

/// 就地写文件的程序：参数只要落到工作区外（绝对路径/盘符/`..`/`~`/`%VAR%`
/// 未展开变量）即判危险；仅相对路径时放行（等价于 write_file 的自治度）。
const FILE_MUTATORS: &[&str] = &[
    "cp", "copy", "mv", "move", "ren", "rename", "xcopy", "robocopy", "tar",
    "unzip", "zip", "7z", "rar", "unrar", "gzip", "gunzip", "bzip2", "bunzip2",
    "xz", "unxz", "zstd", "jar", "ar", "sed", "awk", "gawk", "mawk", "nawk",
    "tee", "patch", "truncate", "install", "ln", "link", "sqlite3", "ffmpeg",
    "magick", "gs", "qpdf", "pdftk", "iconv", "recode", "fmt", "csplit",
    "split", "shuf", "sort", "uniq", "tac", "rev",
];

/// 解释器之外、参数里能藏命令的程序。
fn arg_embeds_command(program: &str, args: &[String]) -> bool {
    match program {
        "find" => args.iter().any(|a| {
            matches!(
                a.as_str(),
                "-exec" | "-execdir" | "-delete" | "-ok" | "-okdir"
                    | "-fls" | "-fprint" | "-fprintf"
            )
        }),
        "fd" => args
            .iter()
            .any(|a| matches!(a.as_str(), "-x" | "--exec" | "-X" | "--exec-batch")),
        "rg" => args
            .iter()
            .any(|a| a == "--pre" || a == "--pre-glob" || a.starts_with("--pre=")),
        "awk" | "gawk" | "mawk" | "nawk" => args.iter().any(|a| a.contains("system(")),
        "perl" => args.iter().any(|a| a.contains("system(") || a.contains("exec(")),
        _ => false,
    }
}

/// 绝对路径/盘符/`..`/`~`/未展开变量 —— 工作区外目标。
fn outside_workspace(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with('\\')
        || token.starts_with('~')
        || token.starts_with('%')
        || token.starts_with('$')
        || token.contains(':')
        || token.split(['/', '\\']).any(|part| part == "..")
}

fn segment_is_dangerous(segment: &str) -> bool {
    let Some(mut tokens) = lex(segment) else { return true };
    // 去掉前导环境赋值与良性包装（`FOO=1 x`、`time make test`）。
    while let Some(first) = tokens.first() {
        let lower = first.to_ascii_lowercase();
        if is_env_assignment(&lower) || BENIGN_PREFIXES.contains(&lower.as_str()) {
            tokens.remove(0);
        } else {
            break;
        }
    }
    let Some(program_token) = tokens.first() else { return false };
    // 段首出现子 shell/分组括号：`(rm -rf /)` —— 按嵌套执行处理。
    if program_token.starts_with('(') || program_token.starts_with('{') {
        return true;
    }
    let Some(program) = program_name(program_token) else {
        // 非裸程序名：路径形态（脚本调用或重定向目标）。工作区相对路径
        // （`./x`、`tools\x`、`logs/out.txt`）与工作区本身同级信任；越界
        // （绝对/盘符/`..`/`~`/变量引用）判危险。
        return outside_workspace(program_token);
    };
    let args = &tokens[1..];
    let program = program.as_str();
    NESTED_EXECUTORS.contains(&program)
        || inline_code(program, args)
        || arg_embeds_command(program, args)
        || subcommand_dangerous(program, args)
        || (DANGEROUS_PROGRAMS.contains(&program) && !dangerous_program_exempt(program, args))
        || (FILE_MUTATORS.contains(&program)
            && args.iter().any(|arg| {
                // 带值 flag（`7z -o../x`）也要查：含 `..`/盘符即越界。
                outside_workspace(arg) || arg.contains("..") || arg.contains(':')
            }))
}

/// `reg query` 是唯一读操作；其余危险程序不看参数。
fn dangerous_program_exempt(program: &str, args: &[String]) -> bool {
    program == "reg" && args.first().is_some_and(|a| a == "query")
}

fn git_dangerous(args: &[String]) -> bool {
    let Some(sub) = args.first() else { return false };
    if sub.starts_with('-') {
        // 全局选项（`-c x=y`、`--exec-path`）可执行/改写 git 行为。
        return true;
    }
    let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    match sub.as_str() {
        // 外发或不可逆改写
        "push" | "rebase" | "filter-branch" | "filter-repo" | "update-ref"
        | "clean" | "svn" | "send-email" | "imap-send" | "daemon" | "rm" => true,
        "gc" => rest.iter().any(|a| a.starts_with("--prune")),
        "reflog" => true,
        "reset" => rest.iter().any(|a| matches!(*a, "--hard" | "--merge" | "--keep")),
        // 丢弃工作区改动：`checkout .`、`checkout -- f`、`restore f`
        "checkout" | "switch" => rest
            .iter()
            .any(|a| matches!(*a, "." | "--" | "--discard-changes" | "-f" | "--force" | "-m" | "--merge")),
        "restore" => {
            // `restore --staged` 只回退 index；带路径的工作区 restore 才丢弃改动。
            if rest
                .iter()
                .any(|a| matches!(*a, "." | "--" | "--discard-changes" | "-f" | "--force" | "-m" | "--merge"))
            {
                true
            } else if rest.iter().any(|a| matches!(*a, "--staged" | "-S")) {
                rest.iter().any(|a| matches!(*a, "--worktree" | "-W"))
            } else {
                rest.iter().any(|a| !a.starts_with('-'))
            }
        }
        "branch" | "tag" => rest
            .iter()
            .any(|a| matches!(*a, "-d" | "-D" | "--delete" | "-f" | "--force")),
        "stash" => rest.iter().any(|a| matches!(*a, "drop" | "clear")),
        "worktree" => rest
            .first()
            .is_some_and(|a| matches!(*a, "remove" | "prune")),
        _ => false,
    }
}

fn subcommand_dangerous(program: &str, args: &[String]) -> bool {
    let first = args.first().map(String::as_str).unwrap_or("");
    let second = args.get(1).map(String::as_str);
    match program {
        "git" => git_dangerous(args),
        "npm" | "pnpm" | "yarn" | "bun" | "corepack" => {
            first == "publish"
                || args
                    .iter()
                    .any(|a| matches!(a.as_str(), "-g" | "--global" | "global" | "deploy"))
        }
        "npx" | "bunx" | "uvx" => first == "publish",
        "cargo" => matches!(first, "publish" | "install"),
        "go" => first == "install",
        "gem" => matches!(first, "install" | "push" | "uninstall" | "cleanup"),
        "composer" => first == "global",
        "poetry" => first == "publish",
        "uv" => {
            first == "publish"
                || (first == "tool" && matches!(second, Some("install" | "uninstall")))
        }
        "dotnet" => {
            (first == "nuget" && matches!(second, Some("push" | "delete")))
                || (first == "tool" && matches!(second, Some("install" | "uninstall" | "update")))
        }
        "mvn" | "mvnw" | "gradle" | "gradlew" | "sbt" | "lein" => args
            .iter()
            .any(|a| matches!(a.as_str(), "deploy" | "publish" | "uploadArchives" | "release")),
        "dart" | "flutter" => args.iter().any(|a| a == "publish"),
        "conda" | "mamba" | "micromamba" => {
            matches!(first, "install" | "remove" | "uninstall" | "clean" | "env")
        }
        "svn" => matches!(
            first,
            "commit" | "ci" | "import" | "copy" | "cp" | "move" | "mv" | "mkdir" | "delete"
                | "del" | "remove" | "rm" | "merge" | "revert" | "resolve" | "patch" | "changelist"
                | "checkout" | "co" | "switch" | "cleanup"
        ),
        "hg" => matches!(
            first,
            "push" | "commit" | "merge" | "rebase" | "strip" | "graft" | "shelve" | "import"
                | "export" | "tag" | "bookmark" | "revert" | "purge" | "clean"
        ),
        "p4" | "bzr" => matches!(
            first,
            "submit" | "delete" | "shelve" | "revert" | "edit" | "move" | "integrate" | "push"
                | "send" | "commit" | "uncommit"
        ),
        "cd" | "chdir" | "pushd" => args.iter().any(|a| outside_workspace(a)),
        _ => false,
    }
}

struct FlagSpec {
    name: &'static str,
    takes_value: bool,
}

const fn flag(name: &'static str) -> FlagSpec {
    FlagSpec { name, takes_value: false }
}

const fn valued(name: &'static str) -> FlagSpec {
    FlagSpec { name, takes_value: true }
}

/// Every flag must be listed; positional arguments may appear anywhere (git
/// accepts `git show HEAD --stat`), but a flag is never treated as a value.
fn args_match(args: &[String], flags: &[FlagSpec], allow_positional: bool) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with('-') {
            // Bundled short flags like `-rn` are accepted when every letter
            // is a listed non-valued flag (`grep -rn`, `ls -la`).
            if let Some(stripped) = arg.strip_prefix('-') {
                if !stripped.starts_with('-') && stripped.len() > 1 {
                    let bundled = stripped.chars().all(|letter| {
                        flags.iter().any(|spec| {
                            !spec.takes_value
                                && spec.name.len() == 2
                                && spec.name.starts_with('-')
                                && spec.name.ends_with(letter)
                        })
                    });
                    if bundled {
                        index += 1;
                        continue;
                    }
                }
            }
            let Some(spec) = flags.iter().find(|spec| spec.name == arg) else {
                return false;
            };
            index += 1;
            if spec.takes_value {
                let Some(value) = args.get(index) else {
                    return false;
                };
                if value.is_empty() || value.starts_with('-') {
                    return false;
                }
                index += 1;
            }
        } else {
            if !allow_positional {
                return false;
            }
            index += 1;
        }
    }
    true
}

const GIT_STATUS: &[FlagSpec] = &[
    flag("--short"),
    flag("-s"),
    flag("--branch"),
    flag("-b"),
    flag("--porcelain"),
    flag("--"),
];
const GIT_DIFF: &[FlagSpec] = &[
    flag("--stat"),
    flag("--shortstat"),
    flag("--numstat"),
    flag("--name-only"),
    flag("--name-status"),
    flag("--cached"),
    flag("--staged"),
    flag("--no-color"),
    flag("--"),
];
const GIT_LOG: &[FlagSpec] = &[
    flag("--oneline"),
    flag("--stat"),
    flag("--graph"),
    flag("--no-color"),
    flag("--name-only"),
    flag("--name-status"),
    flag("-p"),
    valued("-n"),
    valued("--max-count"),
    flag("--"),
];
const GIT_SHOW: &[FlagSpec] = &[
    flag("--stat"),
    flag("--no-color"),
    flag("--name-only"),
    flag("--name-status"),
    flag("--"),
];
const GIT_REV_PARSE: &[FlagSpec] = &[
    flag("--short"),
    flag("--abbrev-ref"),
    flag("--show-toplevel"),
    flag("--is-inside-work-tree"),
    flag("--"),
];
const GIT_LS_FILES: &[FlagSpec] = &[
    flag("--cached"),
    flag("--others"),
    flag("--modified"),
    flag("--deleted"),
    flag("--exclude-standard"),
    flag("--"),
];
const GIT_LS_TREE: &[FlagSpec] = &[
    flag("-r"),
    flag("-l"),
    flag("-t"),
    flag("-d"),
    flag("-z"),
    flag("--name-only"),
    flag("--name-status"),
    flag("--object-only"),
    flag("--full-tree"),
    valued("--abbrev"),
    flag("--"),
];
// `--textconv`/`--filters` 可执行配置的转换程序，batch 模式是交互协议——
// 只有纯对象查询形态只读。
const GIT_CAT_FILE: &[FlagSpec] = &[flag("-t"), flag("-s"), flag("-e"), flag("-p"), flag("--")];
const GIT_REV_LIST: &[FlagSpec] = &[
    flag("--count"),
    valued("-n"),
    valued("--max-count"),
    flag("--oneline"),
    flag("--reverse"),
    flag("--first-parent"),
    flag("--all"),
    flag("--branches"),
    flag("--tags"),
    flag("--merges"),
    flag("--no-merges"),
    flag("--"),
];
const GIT_SHOW_REF: &[FlagSpec] = &[
    flag("--heads"),
    flag("--tags"),
    flag("-s"),
    flag("--hash"),
    flag("-d"),
    flag("--dereference"),
    flag("--"),
];
const GIT_COUNT_OBJECTS: &[FlagSpec] = &[flag("-v"), flag("-H"), flag("--human-readable")];
const GIT_VAR: &[FlagSpec] = &[flag("-l"), flag("--list")];
// sort 的 `-o` 写文件、`--files0-from` 读任意路径清单——只允许纯排序；
// positionals 是待排序文件，读相对路径与 cat 同级。
const SORT_FLAGS: &[FlagSpec] = &[
    flag("-n"),
    flag("-r"),
    flag("-u"),
    flag("-b"),
    flag("-f"),
    flag("-s"),
    flag("-V"),
    flag("-M"),
    flag("-h"),
    flag("-z"),
    valued("-k"),
    valued("-t"),
    flag("--"),
];
const GIT_BLAME: &[FlagSpec] = &[valued("-L"), flag("-n"), flag("--porcelain"), flag("--")];
const GIT_DESCRIBE: &[FlagSpec] = &[flag("--tags"), flag("--always"), flag("--long"), valued("--abbrev")];
const GIT_SHORTLOG: &[FlagSpec] = &[flag("-s"), flag("-n"), flag("--summary"), flag("--")];
const GIT_BRANCH: &[FlagSpec] = &[
    flag("--list"),
    flag("-a"),
    flag("-r"),
    flag("-v"),
    flag("-vv"),
    flag("--all"),
    flag("--remotes"),
    flag("--verbose"),
    flag("--no-color"),
];
const GIT_TAG: &[FlagSpec] = &[
    flag("-l"),
    flag("--list"),
    valued("-n"),
    valued("--sort"),
    valued("--points-at"),
    flag("--"),
];
const GIT_STASH: &[FlagSpec] = &[flag("--stat"), flag("-p"), valued("-n"), flag("--")];
const GIT_GREP: &[FlagSpec] = &[
    flag("-n"),
    flag("-i"),
    flag("-l"),
    flag("-c"),
    flag("-w"),
    flag("-E"),
    flag("-F"),
    flag("--name-only"),
    flag("--"),
];
const RG_FLAGS: &[FlagSpec] = &[
    flag("-n"),
    flag("--line-number"),
    flag("-i"),
    flag("--ignore-case"),
    flag("-l"),
    flag("--files-with-matches"),
    flag("--files"),
    flag("-c"),
    flag("--count"),
    flag("-S"),
    flag("--smart-case"),
    flag("-w"),
    flag("--word-regexp"),
    flag("-F"),
    flag("--fixed-strings"),
    flag("--hidden"),
    flag("--no-heading"),
    valued("-g"),
    valued("--glob"),
    valued("-t"),
    valued("--type"),
    valued("-e"),
    valued("--regexp"),
    valued("-A"),
    valued("-B"),
    valued("-C"),
    valued("-m"),
    valued("--max-count"),
    valued("--max-depth"),
];

fn git_args(args: &[String]) -> bool {
    let Some(subcommand) = args.first() else {
        return false;
    };
    // Global options (`-c`, `--no-pager`, …) can execute or reconfigure git, so
    // the first argument must be a plain subcommand.
    if subcommand.starts_with('-') {
        return false;
    }
    let rest = &args[1..];
    match subcommand.as_str() {
        "status" => args_match(rest, GIT_STATUS, true),
        "diff" => args_match(rest, GIT_DIFF, true),
        "log" => args_match(rest, GIT_LOG, true),
        "show" => args_match(rest, GIT_SHOW, true),
        "rev-parse" => args_match(rest, GIT_REV_PARSE, true),
        "ls-files" => args_match(rest, GIT_LS_FILES, true),
        "ls-tree" => args_match(rest, GIT_LS_TREE, true),
        "cat-file" => args_match(rest, GIT_CAT_FILE, true),
        "rev-list" => args_match(rest, GIT_REV_LIST, true),
        "show-ref" => args_match(rest, GIT_SHOW_REF, true),
        "count-objects" => args_match(rest, GIT_COUNT_OBJECTS, true),
        "var" => args_match(rest, GIT_VAR, true),
        "blame" => args_match(rest, GIT_BLAME, true),
        "describe" => args_match(rest, GIT_DESCRIBE, false),
        "shortlog" => args_match(rest, GIT_SHORTLOG, true),
        // A positional argument would create/rename/delete a branch.
        "branch" => args_match(rest, GIT_BRANCH, false),
        "config" => git_config(rest),
        // `git tag` only in its list forms: bare, or -l/-n filters — a bare
        // positional would create a tag.
        "tag" => rest.is_empty() || (rest.iter().any(|arg| matches!(arg.as_str(), "-l" | "--list" | "-n"))
            && args_match(rest, GIT_TAG, true)),
        // `git remote` only in read forms.
        "remote" => rest.is_empty()
            || (rest.len() == 1 && rest[0] == "-v")
            || (rest.len() == 2 && rest[0] == "get-url"),
        // `git stash` only the read subcommands.
        "stash" => matches!(rest.first().map(String::as_str), Some("list") | Some("show"))
            && args_match(&rest[1..], GIT_STASH, true),
        "grep" => args_match(rest, GIT_GREP, true),
        _ => false,
    }
}

/// `git config` is only allowed in its read forms: at least one read flag, at
/// most one positional key, and only scope flags besides those.
fn git_config(args: &[String]) -> bool {
    const READ: &[&str] = &["--get", "--get-all", "--list", "-l"];
    const SCOPE: &[&str] = &["--global", "--local", "--system"];
    let mut read = false;
    let mut positional = 0;
    for arg in args {
        if READ.contains(&arg.as_str()) {
            read = true;
        } else if SCOPE.contains(&arg.as_str()) {
            continue;
        } else if arg.starts_with('-') {
            return false;
        } else {
            positional += 1;
            if positional > 1 {
                return false;
            }
        }
    }
    read
}

/// File-reading tools must stay inside the workspace: positional arguments
/// are file paths, and an absolute/`..`/drive-qualified path would let a
/// read-only command exfiltrate data `read_file` is scoped away from
/// (`C:\Windows\…`, `/etc/passwd`). Relative names only.
fn positionals_are_relative(args: &[String]) -> bool {
    args.iter().all(|arg| {
        arg.starts_with('-')
            || (!arg.contains(':')
                && !arg.starts_with('/')
                && !arg.starts_with('\\')
                && !arg.split(['/', '\\']).any(|part| part == ".."))
    })
}

/// `where`/`which` take names/patterns only; `/R` and `-` style switches need
/// approval.
fn where_args(args: &[String]) -> bool {
    !args.is_empty() && args.iter().all(|arg| !arg.starts_with('-') && !arg.starts_with('/'))
}

const FD_FLAGS: &[FlagSpec] = &[
    flag("-H"),
    flag("--hidden"),
    flag("-I"),
    flag("--no-ignore"),
    flag("-s"),
    flag("--case-sensitive"),
    flag("-i"),
    flag("--ignore-case"),
    flag("-g"),
    flag("--glob"),
    flag("-l"),
    flag("--list-details"),
    flag("-a"),
    flag("--absolute-path"),
    flag("--full-path"),
    valued("-t"),
    valued("--type"),
    valued("-e"),
    valued("--extension"),
    valued("-d"),
    valued("--max-depth"),
    valued("-j"),
    valued("--threads"),
];
const GREP_FLAGS: &[FlagSpec] = &[
    flag("-n"),
    flag("-i"),
    flag("-r"),
    flag("-R"),
    flag("-l"),
    flag("-c"),
    flag("-w"),
    flag("-v"),
    flag("-E"),
    flag("-F"),
    flag("--color"),
    valued("-A"),
    valued("-B"),
    valued("-C"),
    valued("-m"),
    valued("--include"),
    valued("--exclude"),
];
const CAT_FLAGS: &[FlagSpec] = &[flag("-n"), flag("-b"), flag("-A"), flag("-E"), flag("-T"), flag("-s")];
const HEAD_TAIL_FLAGS: &[FlagSpec] = &[
    valued("-n"),
    valued("-c"),
    flag("-q"),
    flag("-v"),
    flag("-f"),
    flag("--follow"),
];
const WC_FLAGS: &[FlagSpec] = &[flag("-l"), flag("-w"), flag("-c"), flag("-m"), flag("-L")];
const LS_FLAGS: &[FlagSpec] = &[
    flag("-l"),
    flag("-a"),
    flag("-R"),
    flag("-h"),
    flag("-t"),
    flag("-S"),
    flag("-r"),
    flag("-1"),
    flag("-d"),
    flag("-F"),
    flag("--color"),
];
const TREE_FLAGS: &[FlagSpec] = &[
    flag("/F"),
    flag("/A"),
    flag("-a"),
    flag("-d"),
    flag("-f"),
    flag("-i"),
    flag("--dirsfirst"),
    valued("-L"),
    valued("-I"),
    valued("-P"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn safe(command: &str) -> bool {
        is_read_only_safe(command)
    }

    #[test]
    fn allowlisted_git_read_commands_are_safe() {
        for command in [
            "git status",
            "git status --short",
            "git diff",
            "git diff --stat",
            "git diff --name-only src/a.ts",
            "git diff --cached",
            "git log --oneline -n 20",
            "git log --max-count 5 HEAD",
            "git show HEAD --stat",
            "git rev-parse --abbrev-ref HEAD",
            "git ls-files --modified",
            "git blame -L 1,10 src/a.ts",
            "git describe --tags --always",
            "git shortlog -s HEAD",
            "git branch --list",
            "git branch -a",
            "git config --get user.name",
            "git config --global --get user.name",
            "git config --list",
            "git.exe status",
        ] {
            assert!(safe(command), "{command}");
        }
    }

    #[test]
    fn git_write_or_global_option_commands_need_approval() {
        for command in [
            "git",
            "git checkout .",
            "git commit -m x",
            "git push",
            "git fetch",
            "git reset --hard",
            "git clean -fdx",
            "git stash",
            "git -c core.pager=calc diff",
            "git --no-pager diff",
            "git diff --output=x",
            "git diff --output x",
            "git log --output x",
            "git log --format=%H",
            "git branch -D main",
            "git branch new-branch",
            "git config user.name x",
            "git config --global user.name x",
            "git config --unset user.name",
            "git describe HEAD",
            "git status --short=1",
            "git status -sb extra --stat",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn shell_composition_and_expansion_never_parse_as_safe() {
        for command in [
            "git status && del x",
            "git status; del x",
            "git status > out.txt",
            "git status < in.txt",
            "git status\ndel x",
            "git status\tdel x",
            "git status %PATH%",
            "git status ^& del x",
            "git status $(rm -rf .)",
            "git status `rm -rf .`",
            "git status *.rs",
            "git status ~/x",
            "git status !x",
            "git status #x",
            "git status (x)",
            "git status {x}",
            "git status [x]",
            "git status ?",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn program_names_must_be_bare_and_allowlisted() {
        for command in [
            r"C:\Windows\System32\git.exe status",
            "./git status",
            "../git status",
            "tools/git status",
            "npm install",
            "python -c \"print(1)\"",
            "find . -exec rm {} ;",
            "dir",
            "cmd /C git status",
            "powershell -enc AAAA",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn file_reading_tools_only_accept_relative_workspace_paths() {
        for command in [
            "cat src/main.rs",
            "head -n 20 Cargo.toml",
            "tail -f logs/app.log",
            "wc -l src/lib.rs",
            "ls src",
            "tree src",
            "fd -e rs",
            "grep -rn TODO src",
            "echo hello world",
            "pwd",
            "whoami",
            "node --version",
            "python -V",
            "git tag -l",
            "git tag -n 5",
            "git remote -v",
            "git stash list",
            "git grep -n TODO",
        ] {
            assert!(safe(command), "{command}");
        }
        for command in [
            "cat C:/Windows/system.ini",
            "cat /etc/passwd",
            "cat ../outside.txt",
            "head ..\\\\secret.txt",
            "more C:\\\\autoexec.bat",
            "fd -x rm",
            "grep -r x /etc",
            "ls /",
            "tree C:\\\\",
            "echo $PATH",
            "pwd x",
            "node -e print(1)",
            "npm config list",
            "git tag v1.0",
            "git remote add x y",
            "git stash drop",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn rg_and_where_rules_are_flag_scoped() {
        for command in [
            "rg TODO",
            "rg -n TODO src",
            "rg -i --files-with-matches TODO",
            "rg --files",
            "rg -A 2 TODO src",
            "where cmd",
            "where git.exe",
        ] {
            assert!(safe(command), "{command}");
        }
        for command in [
            "rg --pre cat TODO",
            "rg --pre-glob *.rs TODO",
            "rg -z TODO",
            "rg --type-add x:*.x TODO",
            "where",
            "where /R C:\\ x",
            "where -x cmd",
            "rg TODO --pre cat",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn quoted_arguments_group_but_cannot_hide_metacharacters() {
        assert!(safe("git log --oneline -n 20 -- \"main\""));
        assert_eq!(
            parse("git show \"main\""),
            CommandPolicy::ReadOnly { segments: vec![crate::domain::command_policy::ReadOnlySegment { program: "git".into(), args: vec!["show".into(), "main".into()] }] }
        );
        assert!(!safe("git show \"a|b\""));
        assert!(!safe("git show \"unterminated"));
    }

    #[test]
    fn empty_and_oversized_inputs_are_never_safe() {
        for command in ["", "   ", "\n", "\0"] {
            assert!(!safe(command), "{command:?}");
        }
        assert!(!safe(&format!("git status {}", "x".repeat(5000))));
    }

    // ==================== 自主模式风险分类 ====================

    fn dangerous(command: &str) -> bool {
        is_dangerous(command)
    }

    #[test]
    fn everyday_dev_commands_are_auto() {
        for command in [
            "python run_tests.py",
            "python3 -m pytest tests",
            "npm test",
            "npm install",
            "npm run build",
            "pnpm build",
            "yarn lint",
            "cargo test",
            "cargo build --release",
            "go test ./...",
            "dotnet build",
            "mvn package",
            "make test",
            "tsc --noEmit",
            "eslint src",
            "git status",
            "git add .",
            "git commit -m x",
            "git log --oneline -n 5",
            "git merge feature",
            "git checkout -b new-branch",
            "git stash",
            "git stash pop",
            "dir",
            "ls -la src",
            "mkdir out",
            "cd src && npm test",
            "type Cargo.toml",
            "python -V",
            "echo hello",
            r"tools\build.bat",
            "./scripts/check.sh",
        ] {
            assert!(!dangerous(command), "{command}");
        }
    }

    #[test]
    fn destructive_and_remote_commands_are_dangerous() {
        for command in [
            "rm -rf x",
            "rm file.txt",
            "del out.txt",
            "rmdir /s /q dir",
            "rd dir",
            "format c:",
            "shutdown /s",
            "taskkill /pid 1",
            "kill -9 1",
            "sc delete svc",
            "schtasks /delete /tn t",
            "reg add HKLM\\x",
            "reg delete HKCU\\x",
            "netsh advfirewall set allprofiles state off",
            "icacls x /grant y",
            "takeown /f x",
            "curl https://x",
            "wget http://x",
            "ssh host",
            "scp a b",
            "nc -l 80",
            "docker ps",
            "kubectl delete pod x",
            "terraform apply",
            "npm publish",
            "npm install -g x",
            "pnpm add -g y",
            "yarn global add z",
            "cargo publish",
            "cargo install ripgrep",
            "go install tool@latest",
            "pipx install x",
            "apt install x",
            "brew install x",
            "winget install x",
            "choco install x",
            "git push",
            "git push origin main",
            "git reset --hard",
            "git clean -fd",
            "git checkout .",
            "git checkout -- file",
            "git restore a.txt",
            "git branch -D old",
            "git tag -d v1",
            "git stash drop",
            "git stash clear",
            "git rebase main",
            "git filter-branch --all",
            "git rm a.txt",
            "git reflog expire --expire=now --all",
            "git -c core.pager=rm diff",
            "dotnet nuget push x.nupkg",
            "dotnet tool install -g x",
            "mvn deploy",
            "gradle publish",
            "find . -exec rm {} ;",
            "find . -delete",
            "fd -x rm",
            "rg --pre cat x",
            "svn commit",
            "svn checkout url",
            "hg push",
            "sudo ls",
            "runas /user:x cmd",
            "env VAR=1",
            "wmic process list",
            "mysql -u root",
            "psql -c x",
            "redis-cli flushall",
            "printenv",
            "gh release create",
        ] {
            assert!(dangerous(command), "{command}");
        }
    }

    #[test]
    fn composition_and_executors_are_dangerous() {
        for command in [
            "git status && del x",
            "echo ok; rm -rf x",
            "ls | rm",
            "cmd /C dir",
            "cmd.exe /c echo hi",
            "powershell -enc AAAA",
            "pwsh -Command ls",
            "bash -c ls",
            "sh -c 'echo hi'",
            "wsl ls",
            "python -c \"print(1)\"",
            "python3 -c x",
            "node -e \"1\"",
            "node --eval x",
            "ruby -e x",
            "perl -e 'system(1)'",
            "php -r 'echo 1;'",
            "eval ls",
            "exec ls",
            "xargs rm",
            "forfiles /m *.x /c x",
            "env ls",
            "source ~/.bashrc",
            "git status $(rm -rf .)",
            "git status `rm -rf .`",
            "(rm -rf x)",
            "echo x > ../outside.txt",
            "echo x > /etc/x",
            "echo x > C:\\Windows\\x",
            "cat a.txt | tee /tmp/x",
            "cp a ../b",
            "mv a C:/x",
            "tar xf a.tar -C /",
            "7z x a.zip -o../out",
            "sed -i x ../f",
            "cd .. && npm test",
            "cd ../x",
            "awk 'system(\"x\")' f",
            "$TOOLS/evil",
        ] {
            assert!(dangerous(command), "{command}");
        }
    }

    #[test]
    fn auto_but_safe_compositions_pass() {
        for command in [
            "git status && npm test",
            "cargo build && cargo test",
            "ls src | more",
            "echo done > build.log",
            "echo ok > out/result.txt",
            "time make test",
            "FOO=1 npm test",
            "cp a.txt backup/",
            "mv src old-src",
            "mkdir -p out && echo done",
            "git checkout main",
            "git restore --staged a",
            "git diff > changes.diff",
        ] {
            assert!(!dangerous(command), "{command}");
        }
    }

    #[test]
    fn unclassifiable_and_empty_inputs_are_dangerous() {
        for command in ["", "   ", "&&", "|", ";", "git status \"unterminated"] {
            assert!(dangerous(command), "{command:?}");
        }
        assert!(dangerous(&format!("npm test {}", "x".repeat(5000))));
    }

    #[test]
    fn parse_tiers_split_readonly_auto_dangerous() {
        assert_eq!(
            parse("git status"),
            CommandPolicy::ReadOnly { segments: vec![crate::domain::command_policy::ReadOnlySegment { program: "git".into(), args: vec!["status".into()] }] }
        );
        assert_eq!(parse("npm test"), CommandPolicy::Auto);
        assert_eq!(parse("dir"), CommandPolicy::Auto);
        assert_eq!(parse("python run_tests.py"), CommandPolicy::Auto);
        assert_eq!(parse("git push"), CommandPolicy::Dangerous);
        assert_eq!(parse("rm -rf x"), CommandPolicy::Dangerous);
        assert_eq!(parse("cmd /C dir"), CommandPolicy::Dangerous);
    }

    #[test]
    fn pipelines_of_allowlisted_segments_are_readonly() {
        // 截图原案：git ls-tree 管道 sort/head。
        assert!(safe("git ls-tree -r -l HEAD | sort -k4 -n -r | head -n 45"));
        assert!(safe("git status --short | sort"));
        assert!(safe("cat src/main.rs | wc -l"));
        assert!(safe("git status | more"));
        assert!(safe("git ls-tree HEAD | sort -nr"));
        // 单命令是单段管道。
        assert!(safe("git ls-tree HEAD"));
        assert!(safe("git cat-file -p HEAD"));
        assert!(safe("git rev-list --count HEAD"));
        assert!(safe("git show-ref --heads"));
        assert!(safe("git count-objects -v"));
    }

    #[test]
    fn pipelines_reject_unsafe_segments_and_write_caps() {
        // 任一段不在白名单 = 整体拒绝。
        assert!(!safe("git status | rm -rf x"));
        assert!(!safe("echo hi | sh"));
        // sort -o 写文件——管道内外都不放过。
        assert!(!safe("git ls-tree HEAD | sort -o out.txt"));
        assert!(!safe("sort -o out.txt"));
        assert!(!safe("sort --files0-from list.txt"));
        // cat-file 执行向量与 batch 协议模式。
        assert!(!safe("git cat-file --textconv HEAD:file"));
        assert!(!safe("git cat-file --batch"));
        // 段数超限、空段、双管道。
        assert!(!safe("echo a | echo b | echo c | echo d | echo e"));
        assert!(!safe("echo a || echo b"));
        assert!(!safe("| echo a"));
        // 引号内管道不拆分——但元字符黑名单仍然拒收（不做放宽）。
        assert!(!safe("echo \"a|b\""));
        // 重定向/分号等其他 shell 结构依然拒绝。
        assert!(!safe("echo hi > out.txt"));
        assert!(!safe("echo a; echo b"));
        // 排除网络出口子命令。
        assert!(!safe("git ls-remote origin"));
    }

    #[test]
    fn pipeline_segments_parse_into_multi_segment_readonly() {
        match parse("git ls-tree -r HEAD | sort -n | head -n 5") {
            CommandPolicy::ReadOnly { segments } => {
                assert_eq!(segments.len(), 3);
                assert_eq!(segments[0].program, "git");
                assert_eq!(segments[1].program, "sort");
                assert_eq!(segments[2].program, "head");
            }
            other => panic!("expected readonly pipeline, got {other:?}"),
        }
    }
}
