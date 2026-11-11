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

/// `write_file` tool body: create or overwrite a text file inside the workspace.
pub fn write_file(root: &Path, rel: &str, content: &str) -> AppResult<String> {
    if content.len() > MAX_FILE_BYTES as usize {
        return Err("内容超过 256KB 限制".into());
    }
    let file = scoped_write_path(root, rel)?;
    std::fs::write(&file, content).map_err(|_| "文件写入失败".to_string())?;
    Ok(format!("已写入 {}（{} bytes）", rel, content.len()))
}

/// `apply_patch` tool body: exact-match replacement. `before` must occur
/// exactly once in the file; an empty `before` creates a new file with
/// `after` as its content (the target must not exist yet).
pub fn apply_patch(root: &Path, rel: &str, before: &str, after: &str) -> AppResult<String> {
    if after.len() > MAX_FILE_BYTES as usize {
        return Err("内容超过 256KB 限制".into());
    }
    if before.is_empty() {
        let file = scoped_write_path(root, rel)?;
        if file.exists() {
            return Err("目标文件已存在，无法新建".into());
        }
        std::fs::write(&file, after).map_err(|_| "文件写入失败".to_string())?;
        return Ok(format!("已新建 {}（{} bytes）", rel, after.len()));
    }
    let current = read_file(root, rel)?;
    let matches = current.matches(before).count();
    if matches != 1 {
        return Err(format!("补丁定位失败：before 出现 {matches} 次（需要恰好 1 次）"));
    }
    let updated = current.replacen(before, after, 1);
    if updated.len() > MAX_FILE_BYTES as usize {
        return Err("修改后文件超过 256KB 限制".into());
    }
    let file = scoped_write_path(root, rel)?;
    std::fs::write(&file, &updated).map_err(|_| "文件写入失败".to_string())?;
    Ok(format!("已修改 {}（{} → {} bytes）", rel, current.len(), updated.len()))
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
}
