//! Slash-command prompt templates on disk. Commands live under
//! `~/.maestro/commands` (global) and `<workspace>/.maestro/commands`
//! (project); a project file shadows a same-named global one.
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::domain::{AppResult, CommandView, SkillSource};

const MAX_BODY_BYTES: usize = 65536;
const MAX_DESCRIPTION_BYTES: usize = 1024;
const MAX_COMMANDS_PER_ROOT: usize = 256;

/// Command names share the kebab-case rules used by skill directories.
pub fn valid_command_name(name: &str) -> AppResult<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || trimmed.starts_with('-')
        || trimmed.ends_with('-')
        || trimmed.contains("--")
    {
        return Err("命令名称必须为 1-64 位 kebab-case（小写字母、数字、单个连字符）".into());
    }
    Ok(trimmed)
}

/// Minimal frontmatter parser: `name`, `description`, `argumentHint`
/// (also accepts `argument-hint`) followed by the Markdown body.
pub fn parse_command_md(
    content: &str,
    fallback_name: &str,
) -> AppResult<(String, String, Option<String>, String)> {
    if content.len() > MAX_BODY_BYTES + MAX_DESCRIPTION_BYTES + 4096 {
        return Err("命令文件超过大小限制".into());
    }
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut argument_hint = None;
    let mut body_start = 0usize;
    let trimmed = content.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---") {
        let fence_end = rest
            .find("\n---")
            .ok_or("命令文件 frontmatter 未闭合")?;
        for line in rest[..fence_end].lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().trim_matches('"').trim_matches('\'');
            match key.trim() {
                "name" if !value.is_empty() => name = value.to_string(),
                "description" => description = value.to_string(),
                "argumentHint" | "argument-hint" if !value.is_empty() => {
                    argument_hint = Some(value.to_string())
                }
                _ => {}
            }
        }
        body_start = 3 + fence_end + 4;
        if let Some(newline) = trimmed[body_start..].find('\n') {
            body_start += newline + 1;
        } else {
            body_start = trimmed.len();
        }
    }
    let body = trimmed
        .get(body_start..)
        .unwrap_or_default()
        .trim()
        .to_string();
    if body.len() > MAX_BODY_BYTES {
        return Err("命令正文超过 64KB 限制".into());
    }
    if description.len() > MAX_DESCRIPTION_BYTES {
        return Err("命令描述超过 1KB 限制".into());
    }
    Ok((name, description, argument_hint, body))
}

pub fn global_commands_dir(maestro: &Path) -> PathBuf {
    maestro.join("commands")
}

pub fn project_commands_dir(workspace: &Path) -> PathBuf {
    workspace.join(".maestro").join("commands")
}

fn scan_root(root: &Path, source: SkillSource, into: &mut HashMap<String, CommandView>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten().take(MAX_COMMANDS_PER_ROOT) {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let fallback = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok((name, description, argument_hint, _body)) = parse_command_md(&content, &fallback)
        else {
            continue;
        };
        let Ok(name) = valid_command_name(&name) else {
            continue;
        };
        into.insert(
            name.to_string(),
            CommandView {
                name: name.to_string(),
                description,
                argument_hint,
                source,
                path: path.to_string_lossy().into_owned(),
            },
        );
    }
}

/// Merged view: project commands shadow global ones, sorted by name.
pub fn scan(maestro: &Path, workspace: Option<&Path>) -> Vec<CommandView> {
    let mut found = HashMap::new();
    scan_root(&global_commands_dir(maestro), SkillSource::Global, &mut found);
    if let Some(root) = workspace {
        scan_root(&project_commands_dir(root), SkillSource::Project, &mut found);
    }
    let mut views: Vec<_> = found.into_values().collect();
    views.sort_by(|a, b| a.name.cmp(&b.name));
    views
}

fn resolve_path(maestro: &Path, workspace: Option<&Path>, name: &str) -> AppResult<PathBuf> {
    let name = valid_command_name(name)?;
    if let Some(root) = workspace {
        let candidate = project_commands_dir(root).join(format!("{name}.md"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let candidate = global_commands_dir(maestro).join(format!("{name}.md"));
    if candidate.is_file() {
        return Ok(candidate);
    }
    Err(format!("未知命令：{name}"))
}

/// Full body for `/<name>`; the caller expands `$ARGUMENTS`.
pub fn read(maestro: &Path, workspace: Option<&Path>, name: &str) -> AppResult<String> {
    let path = resolve_path(maestro, workspace, name)?;
    let content = fs::read_to_string(&path).map_err(|_| "无法读取命令文件".to_string())?;
    let fallback = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (_name, _description, _hint, body) = parse_command_md(&content, &fallback)?;
    if body.trim().is_empty() {
        return Err("命令正文为空".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let content = "---\nname: review\ndescription: 审查变更\nargumentHint: <路径>\n---\n\n请审查 $ARGUMENTS。\n";
        let (name, description, hint, body) = parse_command_md(content, "fallback").unwrap();
        assert_eq!(name, "review");
        assert_eq!(description, "审查变更");
        assert_eq!(hint.as_deref(), Some("<路径>"));
        assert_eq!(body, "请审查 $ARGUMENTS。");
    }

    #[test]
    fn falls_back_to_file_stem_without_frontmatter() {
        let (name, description, hint, body) =
            parse_command_md("普通正文", "my-command").unwrap();
        assert_eq!(name, "my-command");
        assert!(description.is_empty());
        assert!(hint.is_none());
        assert_eq!(body, "普通正文");
    }

    #[test]
    fn project_commands_shadow_global() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let workspace = base.path().join("ws");
        write(
            &global_commands_dir(&global).join("dup.md"),
            "---\nname: dup\ndescription: global\n---\n\nG",
        );
        write(
            &project_commands_dir(&workspace).join("dup.md"),
            "---\nname: dup\ndescription: project\n---\n\nP",
        );
        write(
            &global_commands_dir(&global).join("only.md"),
            "---\nname: only\ndescription: global only\n---\n\nO",
        );

        let commands = scan(&global, Some(&workspace));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "dup");
        assert_eq!(commands[0].description, "project");
        assert_eq!(commands[0].source, SkillSource::Project);
        assert_eq!(commands[1].name, "only");
        assert_eq!(read(&global, Some(&workspace), "dup").unwrap(), "P");
        assert_eq!(read(&global, Some(&workspace), "only").unwrap(), "O");
    }

    #[test]
    fn rejects_unsafe_names_and_missing_commands() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        for bad in ["../evil", "Bad Name", "", "-lead"] {
            assert!(valid_command_name(bad).is_err());
            assert!(read(&global, None, bad).is_err());
        }
        assert!(read(&global, None, "missing").is_err());
    }

    #[test]
    fn oversized_body_is_rejected() {
        let content = format!("---\nname: big\n---\n\n{}", "x".repeat(MAX_BODY_BYTES + 1));
        assert!(parse_command_md(&content, "big").is_err());
    }
}
