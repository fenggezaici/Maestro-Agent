//! git 只读感知后端：git CLI subprocess + porcelain 解析。
//! 选 CLI 而非 git2 crate——与 agent 已有的 git 白名单同一来源（行为一致）、
//! 零新本地依赖；porcelain v2 为机器解析设计。git 缺失/非 repo 是状态不是错误。
use std::{path::Path, time::Duration};

use serde::Serialize;
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(10);
const DIFF_CAP_BYTES: usize = 512 * 1024;
const OUTPUT_CAP_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSummary {
    /// false 时其余字段无意义——UI 显示"非 git 仓库"占位。
    pub repo: bool,
    pub git_missing: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicted: usize,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitFileEntry {
    pub path: String,
    /// porcelain XY 状态码（如 "M."、".M"、"A."）；untracked 为 "?"。
    pub xy: String,
    pub orig_path: Option<String>,
    pub added: Option<i64>,
    pub removed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub staged: Vec<GitFileEntry>,
    pub unstaged: Vec<GitFileEntry>,
    pub untracked: Vec<GitFileEntry>,
    pub conflicted: Vec<GitFileEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub subject: String,
    /// 装饰引用（分支/tag/HEAD），已拆分。
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiff {
    pub diff: String,
    pub truncated: bool,
    pub binary: bool,
}

async fn run_git(cwd: &Path, args: &[&str], cap: usize) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .arg("--no-pager")
        .arg("-c")
        .arg("core.quotepath=false")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .map_err(|_| format!("git {} 超时（10s）", args.first().unwrap_or(&"")))?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "git-missing".to_string()
            } else {
                format!("git 执行失败：{error}")
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git {} 失败：{}",
            args.first().unwrap_or(&""),
            stderr.trim().chars().take(200).collect::<String>()
        ));
    }
    let mut stdout = output.stdout;
    if stdout.len() > cap {
        stdout.truncate(cap);
    }
    Ok(stdout)
}

/// 仓库探测：非 repo 返回 None（占位渲染）；git 缺失经错误冒泡。
pub async fn detect_root(cwd: &Path) -> Result<Option<String>, String> {
    match run_git(cwd, &["rev-parse", "--show-toplevel"], 64 * 1024).await {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).trim().replace('\\', "/"))),
        Err(error) if error == "git-missing" => Err(error),
        Err(_) => Ok(None),
    }
}

pub async fn summary(cwd: &Path) -> Result<GitSummary, String> {
    let mut out = GitSummary {
        repo: false,
        git_missing: false,
        root: None,
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicted: 0,
        user_name: None,
        user_email: None,
    };
    let root = match detect_root(cwd).await {
        Ok(Some(root)) => root,
        Ok(None) => return Ok(out),
        Err(error) => {
            if error == "git-missing" {
                out.git_missing = true;
                return Ok(out);
            }
            return Err(error);
        }
    };
    out.repo = true;
    out.root = Some(root.clone());
    let root_path = Path::new(&root);
    let bytes = run_git(root_path, &["status", "--porcelain=v2", "--branch", "-z"], OUTPUT_CAP_BYTES).await?;
    let status = parse_porcelain(&bytes);
    let (branch, upstream, ahead, behind) = parse_branch_headers(&bytes);
    out.branch = branch;
    out.upstream = upstream;
    out.ahead = ahead;
    out.behind = behind;
    out.staged = status.staged.len();
    out.unstaged = status.unstaged.len();
    out.untracked = status.untracked.len();
    out.conflicted = status.conflicted.len();
    if let Ok(bytes) = run_git(root_path, &["config", "user.name"], 4096).await {
        let name = String::from_utf8_lossy(&bytes).trim().to_string();
        if !name.is_empty() {
            out.user_name = Some(name);
        }
    }
    if let Ok(bytes) = run_git(root_path, &["config", "user.email"], 4096).await {
        let email = String::from_utf8_lossy(&bytes).trim().to_string();
        if !email.is_empty() {
            out.user_email = Some(email);
        }
    }
    Ok(out)
}

pub async fn status(cwd: &Path) -> Result<Option<GitStatus>, String> {
    let Some(root) = detect_root(cwd).await? else {
        return Ok(None);
    };
    let root_path = Path::new(&root);
    let bytes = run_git(root_path, &["status", "--porcelain=v2", "--branch", "-z"], OUTPUT_CAP_BYTES).await?;
    let mut parsed = parse_porcelain(&bytes);
    // numstat 补行级增删：unstaged 对 working tree、staged 对 index。
    let unstaged_numstat = run_git(root_path, &["diff", "--numstat", "-z"], OUTPUT_CAP_BYTES).await.unwrap_or_default();
    let staged_numstat = run_git(root_path, &["diff", "--cached", "--numstat", "-z"], OUTPUT_CAP_BYTES).await.unwrap_or_default();
    apply_numstat(&mut parsed.unstaged, &parse_numstat(&unstaged_numstat));
    apply_numstat(&mut parsed.staged, &parse_numstat(&staged_numstat));
    Ok(Some(parsed))
}

pub async fn diff(cwd: &Path, path: &str, source: &str) -> Result<Option<GitDiff>, String> {
    let Some(root) = detect_root(cwd).await? else {
        return Ok(None);
    };
    let root_path = Path::new(&root);
    // source 语义：unstaged=工作区 diff、staged=--cached、commit:<hash>=show、
    // branch:<base>=diff base...HEAD；path 为空时省略 `--` 表示整个 commit/对比。
    let mut args: Vec<String> = match source {
        "staged" => vec!["diff".into(), "--cached".into()],
        other if other.starts_with("commit:") => {
            let hash = other.trim_start_matches("commit:");
            vec!["show".into(), "--format=".into(), "--patch".into(), hash.into()]
        }
        other if other.starts_with("branch:") => {
            let base = other.trim_start_matches("branch:");
            vec!["diff".into(), format!("{base}...HEAD")]
        }
        _ => vec!["diff".into()],
    };
    if !path.is_empty() {
        args.push("--".into());
        args.push(path.to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let bytes = run_git(root_path, &refs, DIFF_CAP_BYTES + 1).await.unwrap_or_default();
    let binary = bytes.windows(6).any(|window| window == b"Binary");
    let truncated = bytes.len() > DIFF_CAP_BYTES;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(DIFF_CAP_BYTES)]).into_owned();
    Ok(Some(GitDiff { diff: text, truncated, binary }))
}

pub async fn log(cwd: &Path, limit: usize, skip: usize) -> Result<Option<Vec<GitCommit>>, String> {
    let Some(root) = detect_root(cwd).await? else {
        return Ok(None);
    };
    let limit = limit.clamp(1, 500);
    let format = "--pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e";
    let skip_arg = format!("--skip={skip}");
    let limit_arg = format!("--max-count={limit}");
    let bytes = run_git(Path::new(&root), &["log", format, &limit_arg, &skip_arg], OUTPUT_CAP_BYTES).await?;
    Ok(Some(parse_log(&bytes)))
}

// ---------- 解析（纯函数，表驱动测试） ----------

/// porcelain v2 -z 解析：记录 NUL 分隔；rename（type 2）的下一条记录是原始路径。
pub fn parse_porcelain(bytes: &[u8]) -> GitStatus {
    let mut out = GitStatus::default();
    let records: Vec<&[u8]> = bytes.split(|byte| *byte == 0).filter(|record| !record.is_empty()).collect();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        let line = String::from_utf8_lossy(record).into_owned();
        index += 1;
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("? ") {
            out.untracked.push(entry(rest, "?", None));
            continue;
        }
        if line.starts_with("! ") {
            continue; // ignored 不展示
        }
        if let Some(rest) = line.strip_prefix("u ") {
            // unmerged：u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
            let parts: Vec<&str> = rest.splitn(10, ' ').collect();
            if parts.len() == 10 {
                let xy = parts[0].to_string();
                out.conflicted.push(entry(parts[9], &xy, None));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("1 ") {
            // ordinary：1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path> —— 共 8 字段；
            // path 含空格时 splitn 保证尾字段完整。
            let parts: Vec<&str> = rest.splitn(8, ' ').collect();
            if parts.len() == 8 {
                place(&mut out, parts[0], parts[7], None);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("2 ") {
            // rename/copy：2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>
            // —— 共 9 字段；type 2 记录后紧跟 orig path 记录（-z 模式）。
            let parts: Vec<&str> = rest.splitn(9, ' ').collect();
            if parts.len() == 9 {
                let orig = records.get(index).map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                if orig.is_some() {
                    index += 1;
                }
                place(&mut out, parts[0], parts[8], orig);
            }
            continue;
        }
    }
    out
}

fn entry(path: &str, xy: &str, orig_path: Option<String>) -> GitFileEntry {
    GitFileEntry { path: path.to_string(), xy: xy.to_string(), orig_path, added: None, removed: None }
}

/// X=index（staged 列）Y=worktree（unstaged 列）；'.' 表示无变化。
fn place(out: &mut GitStatus, xy: &str, path: &str, orig_path: Option<String>) {
    let chars: Vec<char> = xy.chars().collect();
    let x = chars.first().copied().unwrap_or('.');
    let y = chars.get(1).copied().unwrap_or('.');
    if x != '.' {
        out.staged.push(entry(path, xy, orig_path.clone()));
    }
    if y != '.' {
        out.unstaged.push(entry(path, xy, orig_path));
    }
}

fn parse_branch_headers(bytes: &[u8]) -> (Option<String>, Option<String>, i64, i64) {
    let (mut branch, mut upstream, mut ahead, mut behind) = (None, None, 0, 0);
    for record in bytes.split(|byte| *byte == 0) {
        let line = String::from_utf8_lossy(record);
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for part in rest.split_whitespace() {
                if let Some(value) = part.strip_prefix('+') {
                    ahead = value.parse().unwrap_or(0);
                } else if let Some(value) = part.strip_prefix('-') {
                    behind = value.parse().unwrap_or(0);
                }
            }
        }
    }
    (branch, upstream, ahead, behind)
}

/// numstat -z：added TAB removed TAB path NUL（binary 两列为 `-` 解析为 None）。
pub fn parse_numstat(bytes: &[u8]) -> Vec<(String, Option<i64>, Option<i64>)> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let line = String::from_utf8_lossy(record);
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?.parse().ok();
            let removed = parts.next()?.parse().ok();
            let path = parts.next()?.to_string();
            if path.is_empty() {
                return None;
            }
            Some((path, added, removed))
        })
        .collect()
}

fn apply_numstat(entries: &mut [GitFileEntry], stats: &[(String, Option<i64>, Option<i64>)]) {
    for entry in entries.iter_mut() {
        if let Some((_, added, removed)) = stats.iter().find(|(path, _, _)| path == &entry.path) {
            entry.added = *added;
            entry.removed = *removed;
        }
    }
}

/// log 记录 RS(0x1e) 分隔、字段 US(0x1f) 分隔：H/P/an/ae/at/s/D。
pub fn parse_log(bytes: &[u8]) -> Vec<GitCommit> {
    bytes
        .split(|byte| *byte == 0x1e)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let text = text.trim_start_matches('\n');
            if text.is_empty() {
                return None;
            }
            let fields: Vec<&str> = text.split('\x1f').collect();
            if fields.len() < 6 {
                return None;
            }
            Some(GitCommit {
                hash: fields[0].to_string(),
                parents: fields[1].split_whitespace().map(str::to_string).collect(),
                author: fields[2].to_string(),
                email: fields[3].to_string(),
                timestamp: fields[4].parse().unwrap_or(0),
                subject: fields[5].to_string(),
                refs: fields
                    .get(6)
                    .map(|decorations| {
                        decorations
                            .split(',')
                            .map(|item| item.trim())
                            .filter(|item| !item.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parses_staged_unstaged_untracked_in_one_pass() {
        let data = b"# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\x001 M. NNNNNN 100644 100644 100644 abc def staged.txt\x001 .M NNNNNN 100644 100644 100644 abc def dirty.txt\0? new.txt\0";
        let parsed = parse_porcelain(data);
        assert_eq!(parsed.staged.len(), 1);
        assert_eq!(parsed.staged[0].path, "staged.txt");
        assert_eq!(parsed.unstaged.len(), 1);
        assert_eq!(parsed.unstaged[0].path, "dirty.txt");
        assert_eq!(parsed.untracked.len(), 1);
        assert_eq!(parsed.untracked[0].path, "new.txt");
        let (branch, upstream, ahead, behind) = parse_branch_headers(data);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(upstream.as_deref(), Some("origin/main"));
        assert_eq!((ahead, behind), (2, 1));
    }

    #[test]
    fn porcelain_mm_lands_in_both_columns() {
        let data = b"1 MM NNNNNN 100644 100644 100644 abc def both.txt\0";
        let parsed = parse_porcelain(data);
        assert_eq!(parsed.staged.len(), 1);
        assert_eq!(parsed.unstaged.len(), 1);
    }

    #[test]
    fn porcelain_rename_carries_orig_path() {
        let data = b"2 R. NNNNNN 100644 100644 100644 abc def R100 new-name.txt\0old-name.txt\0";
        let parsed = parse_porcelain(data);
        assert_eq!(parsed.staged.len(), 1);
        assert_eq!(parsed.staged[0].path, "new-name.txt");
        assert_eq!(parsed.staged[0].orig_path.as_deref(), Some("old-name.txt"));
    }

    #[test]
    fn porcelain_conflict_goes_to_conflicted_bucket() {
        let data = b"u UU NNNNNN 100644 100644 100644 100644 abc def ghi clash.txt\0";
        let parsed = parse_porcelain(data);
        assert_eq!(parsed.conflicted.len(), 1);
        assert_eq!(parsed.conflicted[0].path, "clash.txt");
        assert!(parsed.staged.is_empty() && parsed.unstaged.is_empty());
    }

    #[test]
    fn porcelain_utf8_paths_survive() {
        let data = "1 .M NNNNNN 100644 100644 100644 abc def 目录/文件.txt\0".as_bytes();
        let parsed = parse_porcelain(data);
        assert_eq!(parsed.unstaged[0].path, "目录/文件.txt");
    }

    #[test]
    fn numstat_parses_add_remove_and_binary() {
        let data = b"10\t3\tfile.txt\0-\t-\tbinary.png\0";
        let stats = parse_numstat(data);
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0], ("file.txt".to_string(), Some(10), Some(3)));
        assert_eq!(stats[1], ("binary.png".to_string(), None, None));
    }

    #[test]
    fn log_parses_fields_and_refs() {
        let data = b"abc123\x1fdef456 parent2\x1fAlice\x1fa@b.c\x1f1700000000\x1fsubject line\x1fHEAD -> main, origin/main, tag: v1\x1e";
        let commits = parse_log(data);
        assert_eq!(commits.len(), 1);
        let commit = &commits[0];
        assert_eq!(commit.hash, "abc123");
        assert_eq!(commit.parents, vec!["def456", "parent2"]);
        assert_eq!(commit.author, "Alice");
        assert_eq!(commit.timestamp, 1700000000);
        assert_eq!(commit.refs, vec!["HEAD -> main", "origin/main", "tag: v1"]);
    }

    /// 端到端：tempdir 真实 git repo 验证 CLI 链路（git 缺失时跳过）。
    #[tokio::test]
    async fn real_repo_roundtrip() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.name", "测试"]);
        git(&["config", "user.email", "t@e.st"]);
        std::fs::write(root.join("tracked.txt"), "v1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);
        // unstaged 修改 + staged 新文件 + untracked
        std::fs::write(root.join("tracked.txt"), "v1\nv2\n").unwrap();
        std::fs::write(root.join("staged.txt"), "new\n").unwrap();
        git(&["add", "staged.txt"]);
        std::fs::write(root.join("untracked.txt"), "fresh\n").unwrap();

        let summary_view = summary(root).await.unwrap();
        assert!(summary_view.repo, "应识别为 repo");
        assert_eq!(summary_view.branch.as_deref(), Some("main"));
        assert_eq!(summary_view.user_name.as_deref(), Some("测试"));
        assert_eq!(summary_view.staged, 1);
        assert_eq!(summary_view.unstaged, 1);
        assert_eq!(summary_view.untracked, 1);

        let status_view = status(root).await.unwrap().unwrap();
        assert_eq!(status_view.staged[0].path, "staged.txt");
        assert_eq!(status_view.unstaged[0].path, "tracked.txt");
        assert_eq!(status_view.unstaged[0].added, Some(1));
        assert_eq!(status_view.untracked[0].path, "untracked.txt");

        let diff_view = diff(root, "tracked.txt", "unstaged").await.unwrap().unwrap();
        assert!(diff_view.diff.contains("+v2"), "{}", diff_view.diff);
        assert!(!diff_view.binary);

        let commits = log(root, 10, 0).await.unwrap().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "init");
        assert_eq!(commits[0].author, "测试");

        // 非 repo 目录是状态不是错误
        let plain = tempfile::tempdir().unwrap();
        assert!(!summary(plain.path()).await.unwrap().repo);
        assert!(status(plain.path()).await.unwrap().is_none());
    }
}
