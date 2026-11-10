//! Workspace-scoped read primitives for the model tools: every path must
//! canonicalize under the selected root (rejects `..`, symlink junction
//! escapes, and absolute paths).
use std::path::{Path, PathBuf};

use crate::domain::AppResult;

const MAX_DIR_ENTRIES: usize = 200;
const MAX_FILE_BYTES: u64 = 262144;
const MAX_TEXT_CHARS: usize = 65536;

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

/// `read_file` tool body: bounded UTF-8 text.
pub fn read_file(root: &Path, rel: &str) -> AppResult<String> {
    let file = scoped_path(root, rel)?;
    let meta = std::fs::metadata(&file).map_err(|_| "文件不可读".to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("文件超过 256KB 限制".into());
    }
    let bytes = std::fs::read(&file).map_err(|_| "文件不可读".to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "非 UTF-8 文本文件".to_string())?;
    Ok(text.chars().take(MAX_TEXT_CHARS).collect())
}
