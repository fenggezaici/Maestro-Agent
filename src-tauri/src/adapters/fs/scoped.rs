//! Workspace-scoped read primitives for the model tools: every path must
//! canonicalize under the selected root (rejects `..`, symlink junction
//! escapes, and absolute paths).
use std::path::{Path, PathBuf};

use crate::domain::AppResult;

const MAX_DIR_ENTRIES: usize = 200;
const MAX_FILE_BYTES: u64 = 262144;
const MAX_TEXT_CHARS: usize = 65536;
/// `read_file` line window: `cat -n`-style output never exceeds this many
/// lines or MAX_TEXT_CHARS, whichever comes first.
const MAX_READ_LINES: usize = 2000;
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_SEARCH_VISITED: usize = 8000;
/// Heavy/derived trees a code search never wants — mirrors the ZCode/rg
/// default junk list (`.gitignore` semantics are intentionally not emulated).
const IGNORED_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "node_modules", "target", "dist", "build", "out",
    ".next", ".nuxt", "__pycache__", ".venv", "venv", "vendor", ".idea", ".vscode",
];

/// 敏感文件保护（对齐 Claude Code/ZCode 的默认敏感路径规则）：私钥、凭据
/// 与 dotenv 内容一旦被模型读到就会随 prompt 上行到远端 provider；被写入
/// 则会覆盖/破坏密钥。命中即硬拒绝，不走审批——审批语义无法表达"这条路径
/// 的数据不该离开本机"。示例/模板文件除外（`.env.example` 等本就该入库）。
pub fn is_sensitive_path(rel: &str) -> bool {
    let rel_path = Path::new(rel);
    let name = rel_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.is_empty() {
        return false;
    }
    // .env 及全部变体（生产/本地环境文件），示例与模板例外。
    if name == ".env"
        || (name.starts_with(".env.")
            && !matches!(
                name.as_str(),
                ".env.example" | ".env.sample" | ".env.template" | ".env.defaults" | ".env.schema"
            ))
    {
        return true;
    }
    // 私钥与密钥库。
    if matches!(name.as_str(), "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".keystore")
        || name.ends_with(".jks")
    {
        return true;
    }
    // 凭据文件（含 token 的包管理/容器/云配置）。
    if matches!(
        name.as_str(),
        "credentials" | ".netrc" | ".npmrc" | ".pypirc" | ".dockercfg" | ".htpasswd"
    ) {
        return true;
    }
    // 整目录敏感：.ssh/.gnupg/.aws/.docker/.kube 下任何文件都按凭据处理。
    rel_path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(part)
                if matches!(
                    part.to_string_lossy().to_ascii_lowercase().as_str(),
                    ".ssh" | ".gnupg" | ".aws" | ".docker" | ".kube"
                )
        )
    })
}

fn sensitive_denied() -> String {
    "路径命中敏感文件保护（凭据/私钥/dotenv），禁止读写".into()
}

/// `.maestro/` 是配置区：graph_rules.json、skills、plugins、commands、agents
/// 都住在这里。模型若能写入，就可以改写架构规则自救，或自造 Skill/插件
/// 清单来满足必选扩展依赖——所以整个目录对模型写入一律硬拒绝（规则文件由
/// 「架构沙盘」IPC 落盘，不走本函数）。读取保持允许。
pub fn is_maestro_config_path(rel: &str) -> bool {
    let normalized = rel.trim().replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    normalized == ".maestro" || normalized.starts_with(".maestro/")
}

/// Resolve a workspace-relative path under `root`.
pub fn scoped_path(root: &Path, rel: &str) -> AppResult<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("路径必须在工作区内".into());
    }
    let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let full = if rel.is_empty() {
        base.clone()
    } else {
        base.join(rel_path)
            .canonicalize()
            .map_err(|_| "路径不存在".to_string())?
    };
    if !full.starts_with(&base) {
        return Err("路径必须在工作区内".into());
    }
    Ok(full)
}

/// `list_dir` tool body: bounded JSON entry list.
pub fn list_dir(root: &Path, rel: &str) -> AppResult<String> {
    let dir = scoped_path(root, rel)?;
    let mut entries = Vec::new();
    let mut read = std::fs::read_dir(&dir).map_err(|_| "目录不可读".to_string())?;
    while let Some(Ok(entry)) = read.next() {
        entries.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "dir": entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
        }));
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }
    }
    Ok(serde_json::to_string_pretty(&entries).unwrap_or_default())
}

/// Depth-first file walk bounded by `MAX_SEARCH_VISITED`; results are
/// workspace-relative `/`-joined paths. Symlinks are never traversed —
/// `DirEntry::file_type` reports the link itself, and collected files are
/// re-canonicalized under `base` before being returned.
fn walk_files(base: &Path, out: &mut Vec<String>) {
    fn visit(dir: &Path, base: &Path, visited: &mut usize, out: &mut Vec<String>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        for entry in read.flatten() {
            if *visited >= MAX_SEARCH_VISITED {
                return;
            }
            *visited += 1;
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if !IGNORED_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                        visit(&path, base, visited, out);
                    }
                }
                Ok(kind) if kind.is_file() => {
                    let Ok(canonical) = path.canonicalize() else { continue };
                    if !canonical.starts_with(base) {
                        continue;
                    }
                    if let Ok(rel) = canonical.strip_prefix(base) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
                _ => {}
            }
        }
    }
    let mut visited = 0;
    visit(base, base, &mut visited, out);
}

/// Minimal glob: `*` matches within a path segment, `?` one char, `**` spans
/// segments. A pattern without `/` matches the file name at any depth.
fn glob_match(pattern: &str, path: &str) -> bool {
    fn seg_match(pat: &[char], text: &[char]) -> bool {
        match (pat.first(), text.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                seg_match(&pat[1..], text) || (!text.is_empty() && seg_match(pat, &text[1..]))
            }
            (Some('?'), Some(_)) => seg_match(&pat[1..], &text[1..]),
            (Some(a), Some(b)) => a == b && seg_match(&pat[1..], &text[1..]),
            _ => false,
        }
    }
    fn parts_match(pats: &[&str], segs: &[&str]) -> bool {
        match (pats.first(), segs.first()) {
            (None, None) => true,
            (Some(&"**"), _) => {
                (0..=segs.len()).any(|skip| parts_match(&pats[1..], &segs[skip..]))
            }
            (Some(p), Some(s)) => {
                seg_match(&p.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>())
                    && parts_match(&pats[1..], &segs[1..])
            }
            _ => false,
        }
    }
    if pattern.is_empty() {
        return false;
    }
    let pats: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    if pats.len() == 1 {
        // Basename glob — `*.rs` means "any .rs file, any depth".
        return segs.last().is_some_and(|name| {
            seg_match(&pattern.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>())
        });
    }
    parts_match(&pats, &segs)
}

/// `glob_files` tool body: workspace-relative paths matching the pattern.
pub fn glob_files(root: &Path, pattern: &str) -> AppResult<String> {
    let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let mut files = Vec::new();
    walk_files(&base, &mut files);
    let mut hits: Vec<String> = files
        .into_iter()
        .filter(|rel| glob_match(&pattern.replace('\\', "/"), rel))
        .collect();
    hits.sort();
    let truncated = hits.len() > MAX_SEARCH_RESULTS;
    hits.truncate(MAX_SEARCH_RESULTS);
    let mut body = hits.join("\n");
    if body.is_empty() {
        body = "（无匹配文件）".into();
    } else if truncated {
        body.push_str(&format!("\n…已截断，仅显示前 {MAX_SEARCH_RESULTS} 个结果"));
    }
    Ok(body)
}

/// `grep_files` tool body: regex line search across text files. `glob` narrows
/// the file set (`src/**/*.rs`); `ignore_case` folds case; `context_lines`
/// (clamped to 0–5) prints that many surrounding lines around every match.
/// Output follows grep conventions — `path:行号: 内容` for matches,
/// `path-行号- 内容` for context, `--` between non-contiguous blocks — and
/// stays bounded at MAX_SEARCH_RESULTS matched lines / MAX_TEXT_CHARS output.
/// An invalid pattern is an error reported back to the model (it can fix and
/// retry), never a silent fallback to literal matching.
pub fn grep_files(
    root: &Path,
    query: &str,
    glob: Option<&str>,
    ignore_case: bool,
    context_lines: u32,
) -> AppResult<String> {
    if query.is_empty() {
        return Err("grep_files 需要非空 query".into());
    }
    let pattern = regex::RegexBuilder::new(query)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|error| format!("正则表达式无效：{error}"))?;
    let context = context_lines.min(5) as usize;
    let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let mut files = Vec::new();
    walk_files(&base, &mut files);
    let mut body = String::new();
    let mut matched = 0usize;
    let mut truncated = false;
    let mut wrote_block = false;
    'files: for rel in files {
        if let Some(glob_pattern) = glob {
            if !glob_match(&glob_pattern.replace('\\', "/"), &rel) {
                continue;
            }
        }
        let path = base.join(&rel);
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if bytes.iter().take(4096).any(|b| *b == 0) {
            continue; // binary file
        }
        let Ok(text) = String::from_utf8(bytes) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let remaining = MAX_SEARCH_RESULTS.saturating_sub(matched);
        if remaining == 0 {
            truncated = true;
            break;
        }
        // Take one past the remaining budget so truncation is detected in the
        // same single pass instead of rescanning the file.
        let mut hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| pattern.is_match(line).then_some(index))
            .take(remaining + 1)
            .collect();
        if hits.len() > remaining {
            truncated = true;
            hits.truncate(remaining);
        }
        if hits.is_empty() {
            continue;
        }
        let hitset: std::collections::HashSet<usize> = hits.iter().copied().collect();
        // Merge context-expanded ranges into contiguous blocks: a hit at line
        // `i` covers [i-context, i+context]; overlapping or touching ranges
        // form one block, `--` separates the rest (grep -C semantics).
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for &hit in &hits {
            let start = hit.saturating_sub(context);
            let end = (hit + context + 1).min(lines.len());
            match blocks.last_mut() {
                Some((_, bend)) if start <= *bend => *bend = end.max(*bend),
                _ => blocks.push((start, end)),
            }
        }
        for (start, end) in blocks {
            if wrote_block {
                body.push_str("--\n");
            }
            wrote_block = true;
            for (offset, line) in lines[start..end].iter().enumerate() {
                let index = start + offset;
                let trimmed: String = line.trim().chars().take(200).collect();
                let lineno = index + 1;
                if hitset.contains(&index) {
                    body.push_str(&format!("{rel}:{lineno}: {trimmed}\n"));
                } else {
                    body.push_str(&format!("{rel}-{lineno}- {trimmed}\n"));
                }
                if body.len() >= MAX_TEXT_CHARS {
                    truncated = true;
                    break 'files;
                }
            }
        }
        matched += hits.len();
    }
    if matched == 0 {
        return Ok("（无匹配行）".into());
    }
    while body.ends_with('\n') {
        body.pop();
    }
    if truncated {
        body.push_str(&format!("\n…已截断，仅显示前 {MAX_SEARCH_RESULTS} 条匹配及其上下文"));
    }
    Ok(body)
}

/// `read_file` tool body with line numbers (`cat -n` style). `offset` is the
/// 1-based first line (default 1), `limit` caps at MAX_READ_LINES (default
/// full). Output stays under MAX_TEXT_CHARS with a truncation notice.
pub fn read_file_lines(
    root: &Path,
    rel: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> AppResult<String> {
    let text = read_file_full(root, rel)?;
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = offset.unwrap_or(1).max(1) as usize;
    let take = limit.unwrap_or(MAX_READ_LINES as u64).min(MAX_READ_LINES as u64) as usize;
    if start > total && total > 0 {
        return Err(format!("offset {start} 超出文件范围（共 {total} 行）"));
    }
    let mut out = String::new();
    let mut rendered = 0usize;
    for (index, line) in lines.iter().enumerate().skip(start - 1).take(take) {
        let row = format!("{:>6}\t{}\n", index + 1, line);
        if out.len() + row.len() > MAX_TEXT_CHARS {
            out.push_str(&format!("…输出达到字符上限（共 {total} 行，已显示 {rendered} 行）"));
            return Ok(out);
        }
        out.push_str(&row);
        rendered += 1;
    }
    if start - 1 + rendered < total {
        out.push_str(&format!("…还有 {} 行未显示（offset 续读）", total - start + 1 - rendered));
    }
    Ok(out)
}

/// `read_file` tool body: bounded UTF-8 text. 模型侧读——截断时附提示，
/// 模型知道内容不完整而改用 offset/limit 分段。
pub fn read_file(root: &Path, rel: &str) -> AppResult<String> {
    let text = read_file_full(root, rel)?;
    if text.chars().count() <= MAX_TEXT_CHARS {
        return Ok(text);
    }
    let total = text.chars().count();
    let mut out: String = text.chars().take(MAX_TEXT_CHARS).collect();
    out.push_str(&format!(
        "…（文件共 {total} 字符，仅显示前 {MAX_TEXT_CHARS} 字符，请用 offset/limit 或 grep 分段读取）"
    ));
    Ok(out)
}

/// 内部完整读：与 `read_file` 相同的安全检查（sensitive path、工作区边界、
/// 256KB 硬上限、UTF-8），但**不做** 64KB 截断。补丁基准、撤销快照、
/// 审批预览等必须拿到真实全文——否则会把截断内容写回磁盘丢数据。
pub fn read_file_full(root: &Path, rel: &str) -> AppResult<String> {
    if is_sensitive_path(rel) {
        return Err(sensitive_denied());
    }
    let file = scoped_path(root, rel)?;
    let meta = std::fs::metadata(&file).map_err(|_| "文件不可读".to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("文件超过 256KB 限制".into());
    }
    let bytes = std::fs::read(&file).map_err(|_| "文件不可读".to_string())?;
    String::from_utf8(bytes).map_err(|_| "非 UTF-8 文本文件".to_string())
}

/// Resolve a workspace-relative write target. The parent directory must
/// already exist inside `root` (writes never create directories or escape
/// through symlinked parents); an existing target must canonicalize inside
/// `root` as well, so `fs::write` cannot follow a symlink out of the workspace.
pub fn scoped_write_path(root: &Path, rel: &str) -> AppResult<PathBuf> {
    let rel_path = Path::new(rel);
    if rel.is_empty()
        || rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("路径必须在工作区内".into());
    }
    if is_maestro_config_path(rel) {
        return Err(".maestro/ 为扩展与架构规则配置区，模型不可写入；请在扩展页或架构沙盘中调整".into());
    }
    let file_name = rel_path
        .file_name()
        .filter(|name| {
            let text = name.to_string_lossy();
            !text.is_empty() && !text.contains(['/', '\\', ':'])
        })
        .ok_or_else(|| "非法文件名".to_string())?;
    let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let parent_full = if parent.as_os_str().is_empty() {
        base.clone()
    } else {
        base.join(parent)
            .canonicalize()
            .map_err(|_| "父目录不存在".to_string())?
    };
    if !parent_full.is_dir() || !parent_full.starts_with(&base) {
        return Err("路径必须在工作区内".into());
    }
    let mut full = parent_full.join(file_name);
    if full.exists() {
        full = full
            .canonicalize()
            .map_err(|_| "无法解析写入路径".to_string())?;
        if !full.starts_with(&base) {
            return Err("路径必须在工作区内".into());
        }
    }
    Ok(full)
}

/// A completed workspace write: the model-facing summary plus the
/// before/after contents the UI needs to render a mini diff.
pub struct WriteReport {
    pub path: String,
    pub summary: String,
    pub before: String,
    pub after: String,
}

/// Remove a workspace file; used only by the explicit undo command.
pub fn remove_file(root: &Path, rel: &str) -> AppResult<()> {
    if is_sensitive_path(rel) {
        return Err(sensitive_denied());
    }
    let file = scoped_write_path(root, rel)?;
    if file.is_file() { std::fs::remove_file(&file).map_err(|_| "文件删除失败".to_string())?; }
    Ok(())
}

/// `write_file` tool body: create or overwrite a text file inside the workspace.
pub fn write_file(root: &Path, rel: &str, content: &str) -> AppResult<WriteReport> {
    if is_sensitive_path(rel) {
        return Err(sensitive_denied());
    }
    if content.len() > MAX_FILE_BYTES as usize {
        return Err("内容超过 256KB 限制".into());
    }
    let file = scoped_write_path(root, rel)?;
    let before = if file.is_file() {
        read_file_full(root, rel).unwrap_or_default()
    } else {
        String::new()
    };
    std::fs::write(&file, content).map_err(|_| "文件写入失败".to_string())?;
    Ok(WriteReport {
        path: rel.to_string(),
        summary: format!("已写入 {}（{} bytes）", rel, content.len()),
        before,
        after: content.to_string(),
    })
}

/// `apply_patch` tool body: exact-match replacement. `before` must occur
/// exactly once in the file; an empty `before` creates a new file with
/// `after` as its content (the target must not exist yet).
pub fn apply_patch(root: &Path, rel: &str, before: &str, after: &str) -> AppResult<WriteReport> {
    apply_patch_multi(root, rel, &[(before, after)])
}

/// Multi-hunk patch: each `(before, after)` is applied in order against the
/// evolving text — every `before` must occur exactly once at its turn. All
/// hunks validate before the single final write, so a bad hunk never leaves
/// a half-patched file (ZCode-style atomic edits).
pub fn apply_patch_multi(root: &Path, rel: &str, hunks: &[(&str, &str)]) -> AppResult<WriteReport> {
    if is_sensitive_path(rel) {
        return Err(sensitive_denied());
    }
    if hunks.is_empty() {
        return Err("补丁为空".into());
    }
    if hunks.iter().any(|(_, after)| after.len() > MAX_FILE_BYTES as usize) {
        return Err("内容超过 256KB 限制".into());
    }
    // Empty `before` means "create the file" — only allowed as the sole hunk.
    if let Some((_, after)) = hunks.iter().find(|(before, _)| before.is_empty()) {
        if hunks.len() > 1 {
            return Err("新建文件（before 为空）只能作为唯一 hunk".into());
        }
        let file = scoped_write_path(root, rel)?;
        if file.exists() {
            return Err("目标文件已存在，无法新建".into());
        }
        std::fs::write(&file, after).map_err(|_| "文件写入失败".to_string())?;
        return Ok(WriteReport {
            path: rel.to_string(),
            summary: format!("已新建 {}（{} bytes）", rel, after.len()),
            before: String::new(),
            after: after.to_string(),
        });
    }
    let original = read_file_full(root, rel)?;
    let mut updated = original.clone();
    for (index, (before, after)) in hunks.iter().enumerate() {
        let matches = updated.matches(before).count();
        if matches != 1 {
            return Err(format!(
                "补丁第 {} 个 hunk 定位失败：before 出现 {matches} 次（需要恰好 1 次）",
                index + 1
            ));
        }
        updated = updated.replacen(before, after, 1);
    }
    if updated.len() > MAX_FILE_BYTES as usize {
        return Err("修改后文件超过 256KB 限制".into());
    }
    let file = scoped_write_path(root, rel)?;
    std::fs::write(&file, &updated).map_err(|_| "文件写入失败".to_string())?;
    Ok(WriteReport {
        path: rel.to_string(),
        summary: format!(
            "已修改 {}（{} 个 hunk，{} → {} bytes）",
            rel,
            hunks.len(),
            original.len(),
            updated.len()
        ),
        before: original,
        after: updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_paths_stay_inside_the_workspace() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        for rel in ["../escape.txt", "/abs.txt", r"C:\win.txt", "", "a/../b.txt", r"a\..\b.txt"] {
            assert!(scoped_write_path(root, rel).is_err(), "{rel}");
        }
        // Parent directories are never created implicitly.
        assert!(scoped_write_path(root, "sub/file.txt").is_err());
        std::fs::create_dir_all(root.join("sub")).unwrap();
        assert!(scoped_write_path(root, "sub/file.txt").is_ok());
    }

    #[test]
    fn write_file_and_apply_patch_roundtrip() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        write_file(root, "a.txt", "hello world").unwrap();
        apply_patch(root, "a.txt", "world", "rust").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "hello rust"
        );
        // An ambiguous before must never apply.
        write_file(root, "b.txt", "x x").unwrap();
        assert!(apply_patch(root, "b.txt", "x", "y").is_err());
        // Empty before creates a new file but never overwrites.
        apply_patch(root, "c.txt", "", "new").unwrap();
        assert!(apply_patch(root, "c.txt", "", "again").is_err());
        // Oversized content is refused.
        assert!(write_file(root, "big.txt", &"x".repeat(262145)).is_err());
    }

    #[test]
    fn read_tools_still_confine_paths() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        std::fs::write(root.join("a.txt"), "hi").unwrap();
        assert!(read_file(root, "a.txt").is_ok());
        assert!(read_file(root, "../a.txt").is_err());
        assert!(list_dir(root, "..").is_err());
    }

    #[test]
    fn apply_patch_on_a_large_file_preserves_the_tail() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        // ~100KB 文件：头部一小段 + 远超 64KB 的尾部。
        let tail = "t".repeat(80_000);
        std::fs::write(root.join("big.txt"), format!("HEADER\n{tail}\nFOOTER")).unwrap();
        let report = apply_patch(root, "big.txt", "HEADER\n", "PATCHED\n").unwrap();
        assert!(report.after.ends_with(&format!("{tail}\nFOOTER")));
        let on_disk = std::fs::read_to_string(root.join("big.txt")).unwrap();
        assert!(on_disk.ends_with("FOOTER"));
        assert_eq!(on_disk.len(), report.after.len());
    }

    #[test]
    fn write_file_before_snapshot_keeps_the_full_large_file() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let content = "x".repeat(100_000);
        std::fs::write(root.join("big.txt"), &content).unwrap();
        let report = write_file(root, "big.txt", "new").unwrap();
        assert_eq!(report.before, content);
    }

    #[test]
    fn read_file_lines_reports_the_true_line_count() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        // 构造总行数超过 64KB 渲染上限的文件：每行 ~60 字符 × 1400 行 ≈ 84KB。
        let text: String = (1..=1400)
            .map(|n| format!("line-{n}-{}\n", "v".repeat(50)))
            .collect();
        std::fs::write(root.join("many.txt"), &text).unwrap();
        let out = read_file_lines(root, "many.txt", None, None).unwrap();
        assert!(out.contains("共 1400 行"), "{out}");
        // offset 续读能看到尾部行。
        let tail = read_file_lines(root, "many.txt", Some(1400), Some(1)).unwrap();
        assert!(tail.contains("1400\tline-1400"), "{tail}");
    }

    #[test]
    fn read_file_marks_truncated_content() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        std::fs::write(root.join("big.txt"), "x".repeat(100_000)).unwrap();
        let out = read_file(root, "big.txt").unwrap();
        assert!(out.contains("仅显示前"), "{out}");
        assert!(out.contains("offset"), "{out}");
        // read_file_full 返回完整内容。
        assert_eq!(read_file_full(root, "big.txt").unwrap().len(), 100_000);
    }

    #[test]
    fn grep_files_supports_regex_alternation_and_classes() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        std::fs::write(
            root.join("a.rs"),
            "fn read_file() {}\nlet x = 1;\nfn write_file() {}\nlet y = 2;\nfn other() {}\n",
        )
        .unwrap();
        let out = grep_files(root, "fn (read|write)_file", None, false, 0).unwrap();
        assert!(out.contains("a.rs:1: fn read_file() {}"), "{out}");
        assert!(out.contains("a.rs:3: fn write_file() {}"), "{out}");
        assert!(!out.contains("other"), "{out}");
        // `.` 是正则元字符：a.c 同时命中 abc 与字面 a.c。
        std::fs::write(root.join("b.txt"), "abc\na.c\n").unwrap();
        let out = grep_files(root, "a.c", None, false, 0).unwrap();
        assert!(out.contains("b.txt:1: abc"), "{out}");
        assert!(out.contains("b.txt:2: a.c"), "{out}");
    }

    #[test]
    fn grep_files_invalid_regex_is_an_error_not_a_literal_fallback() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        std::fs::write(root.join("a.txt"), "anything\n").unwrap();
        let error = grep_files(root, "(", None, false, 0).unwrap_err();
        assert!(error.contains("正则表达式无效"), "{error}");
    }

    #[test]
    fn grep_files_ignore_case_and_glob_filter() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        std::fs::write(root.join("a.rs"), "fn read_file() {}\n").unwrap();
        std::fs::write(root.join("b.txt"), "read_file mentioned\n").unwrap();
        let out = grep_files(root, "READ_FILE", None, true, 0).unwrap();
        assert!(out.contains("a.rs:1:"), "{out}");
        let only_txt = grep_files(root, "read_file", Some("*.txt"), false, 0).unwrap();
        assert!(only_txt.contains("b.txt:1:"), "{only_txt}");
        assert!(!only_txt.contains("a.rs"), "{only_txt}");
        let no_hit = grep_files(root, "read_file", Some("*.md"), false, 0).unwrap();
        assert_eq!(no_hit, "（无匹配行）");
    }

    #[test]
    fn grep_files_context_lines_merge_overlapping_blocks() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        // 命中第 1、3 行，context=1 的扩展区间 [1-2] 与 [2-4] 相触合并为单块，
        // 中间不出现 `--` 分隔；与第二个远处命中之间才有分隔。
        std::fs::write(
            root.join("a.txt"),
            "hit-one\nctx\nhit-two\nctx\nfar\nfar\nfar\nhit-three\n",
        )
        .unwrap();
        let out = grep_files(root, "^hit-", None, false, 1).unwrap();
        assert!(out.contains("a.txt:1: hit-one"), "{out}");
        assert!(out.contains("a.txt-2- ctx"), "{out}");
        assert!(out.contains("a.txt:3: hit-two"), "{out}");
        assert!(out.contains("a.txt:8: hit-three"), "{out}");
        assert!(out.contains("--\n"), "{out}");
        // context=0 时无上下文行也无分隔。
        let plain = grep_files(root, "^hit-", None, false, 0).unwrap();
        assert!(!plain.contains("-2-"), "{plain}");
        // context 超过 5 被钳制。
        let clamped = grep_files(root, "^hit-one", None, false, 99).unwrap();
        assert!(clamped.lines().count() <= 8, "{clamped}");
    }
}
