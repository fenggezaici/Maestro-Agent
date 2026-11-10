//! Skill discovery and management. Skills live on disk under `~/.maestro/skills`
//! (global) and `<workspace>/.maestro/skills` (project); nothing is stored in SQLite.
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use serde::Serialize;

use crate::model::AppResult;

const MAX_BODY_BYTES: usize = 131072;
const MAX_DESCRIPTION_BYTES: usize = 1024;
const MAX_SKILLS_PER_ROOT: usize = 256;
const CLONE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Global,
    Project,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillImportOutcome {
    pub name: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct SkillEntry {
    name: String,
    description: String,
    body: String,
    path: PathBuf,
    source: SkillSource,
}

impl SkillEntry {
    fn view(&self) -> SkillView {
        SkillView {
            name: self.name.clone(),
            description: self.description.clone(),
            source: self.source,
            path: self.path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: String,
    pub source_url: String,
    pub install_payload: String,
}

/// A `key: value` name must be a safe kebab-case directory and tool identifier.
pub fn valid_skill_name(name: &str) -> AppResult<&str> {
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
        return Err("技能名称必须为 1-64 位 kebab-case（小写字母、数字、单个连字符）".into());
    }
    Ok(trimmed)
}

/// Minimal YAML frontmatter parser: `---` fenced block with single-line
/// `name:` / `description:` keys followed by the Markdown body.
pub fn parse_skill_md(content: &str, fallback_name: &str) -> AppResult<(String, String, String)> {
    if content.len() > MAX_BODY_BYTES + MAX_DESCRIPTION_BYTES + 4096 {
        return Err("SKILL.md 超过大小限制".into());
    }
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut body_start = 0usize;
    let trimmed = content.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---") {
        let fence_end = rest
            .find("\n---")
            .ok_or("SKILL.md frontmatter 未闭合")?;
        let frontmatter = &rest[..fence_end];
        for line in frontmatter.lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = value.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(value) = line.strip_prefix("description:") {
                description = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
            }
        }
        body_start = 3 + fence_end + 4;
        // Skip the remainder of the closing fence line.
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
        return Err("SKILL.md 正文超过 128KB 限制".into());
    }
    if description.len() > MAX_DESCRIPTION_BYTES {
        return Err("SKILL.md description 超过 1KB 限制".into());
    }
    Ok((name, description, body))
}

fn scan_root(root: &Path, source: SkillSource, into: &mut HashMap<String, SkillEntry>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten().take(MAX_SKILLS_PER_ROOT) {
        let skill_md = entry.path().join("SKILL.md");
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) || !skill_md.is_file() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(content) = fs::read_to_string(&skill_md) else {
            continue;
        };
        let Ok((name, description, body)) = parse_skill_md(&content, &dir_name) else {
            continue;
        };
        let Ok(name) = valid_skill_name(&name) else {
            continue;
        };
        into.insert(
            name.to_string(),
            SkillEntry {
                name: name.to_string(),
                description,
                body,
                path: skill_md,
                source,
            },
        );
    }
}

#[derive(Default)]
pub struct SkillStore {
    skills: Mutex<HashMap<String, SkillEntry>>,
}

impl SkillStore {
    /// Project skills shadow global skills with the same name.
    pub fn scan(&self, global_dir: &Path, workspace_dir: Option<&Path>) -> Vec<SkillView> {
        let mut found = HashMap::new();
        scan_root(&global_dir.join("skills"), SkillSource::Global, &mut found);
        if let Some(workspace) = workspace_dir {
            scan_root(
                &workspace.join(".maestro").join("skills"),
                SkillSource::Project,
                &mut found,
            );
        }
        *self.skills.lock().unwrap() = found.clone();
        let mut views: Vec<_> = found.values().map(SkillEntry::view).collect();
        views.sort_by(|a, b| a.name.cmp(&b.name));
        views
    }

    /// Ensure a rescan then return the full body the model asked for.
    pub fn body(
        &self,
        name: &str,
        global_dir: &Path,
        workspace_dir: Option<&Path>,
    ) -> AppResult<String> {
        let known = {
            let skills = self.skills.lock().unwrap();
            skills.get(name).map(|entry| entry.body.clone())
        };
        if let Some(body) = known {
            return Ok(body);
        }
        // Rescan in case the file appeared after the last listing.
        self.scan(global_dir, workspace_dir);
        self.skills
            .lock()
            .unwrap()
            .get(name)
            .map(|entry| entry.body.clone())
            .ok_or_else(|| format!("未知技能：{name}"))
    }

    pub fn create_new(&self, global_dir: &Path, name: &str, description: &str) -> AppResult<SkillView> {
        let name = valid_skill_name(name)?;
        let description = description.trim();
        if description.len() > MAX_DESCRIPTION_BYTES {
            return Err("技能描述超过 1KB 限制".into());
        }
        let dir = global_dir.join("skills").join(name);
        if dir.exists() {
            return Err(format!("技能 {name} 已存在"));
        }
        fs::create_dir_all(&dir).map_err(|_| "无法创建技能目录".to_string())?;
        let path = dir.join("SKILL.md");
        let template = format!(
            "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\n在这里编写技能的指令内容。模型可通过 load_skill 加载本技能。\n"
        );
        fs::write(&path, template).map_err(|_| "无法写入 SKILL.md".to_string())?;
        Ok(SkillView {
            name: name.to_string(),
            description: description.to_string(),
            source: SkillSource::Global,
            path: path.to_string_lossy().into_owned(),
        })
    }

    /// Copy a directory containing SKILL.md (or a SKILL.md file itself) into the
    /// global skills directory. The frontmatter name decides the target folder.
    /// A directory without SKILL.md is treated as a batch parent: every direct
    /// child directory containing SKILL.md is imported independently.
    pub fn import_from_path(
        &self,
        global_dir: &Path,
        source_path: &str,
    ) -> AppResult<Vec<SkillImportOutcome>> {
        let source = Path::new(source_path)
            .canonicalize()
            .map_err(|_| "导入路径不存在".to_string())?;
        let skill_md = if source.is_dir() {
            source.join("SKILL.md")
        } else {
            source.clone()
        };
        if skill_md.file_name().map(|n| n != "SKILL.md").unwrap_or(true) || !skill_md.is_file() {
            let mut outcomes = self.import_children(global_dir, &source)?;
            if outcomes.is_empty() {
                return Err("导入路径必须是包含 SKILL.md 的目录、SKILL.md 文件，或含技能子目录的父目录".into());
            }
            outcomes.sort_by(|a, b| a.name.cmp(&b.name));
            return Ok(outcomes);
        }
        self.import_single(global_dir, &skill_md, &source)
            .map(|view| {
                vec![SkillImportOutcome {
                    name: view.name,
                    status: "imported".into(),
                    error: None,
                }]
            })
    }

    /// Import every direct child directory of `parent` that contains SKILL.md.
    /// Each child is independent: one bad or duplicate skill does not stop the rest.
    fn import_children(&self, global_dir: &Path, parent: &Path) -> AppResult<Vec<SkillImportOutcome>> {
        if !parent.is_dir() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(parent).map_err(|_| "无法读取导入目录".to_string())?;
        let mut outcomes = Vec::new();
        for entry in entries.flatten() {
            let skill_md = entry.path().join("SKILL.md");
            if !entry.path().is_dir() || !skill_md.is_file() {
                continue;
            }
            let label = entry.file_name().to_string_lossy().into_owned();
            outcomes.push(match self.import_single(global_dir, &skill_md, &entry.path()) {
                Ok(view) => SkillImportOutcome { name: view.name, status: "imported".into(), error: None },
                Err(reason) if reason.starts_with("技能") && reason.contains("已存在") => {
                    SkillImportOutcome { name: label, status: "skipped".into(), error: Some(reason) }
                }
                Err(reason) => SkillImportOutcome { name: label, status: "failed".into(), error: Some(reason) },
            });
        }
        Ok(outcomes)
    }

    /// Copy one SKILL.md into the global skills directory; shared by single and batch imports.
    fn import_single(&self, global_dir: &Path, skill_md: &Path, source: &Path) -> AppResult<SkillView> {
        let content = fs::read_to_string(skill_md).map_err(|_| "无法读取 SKILL.md".to_string())?;
        let fallback = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "imported-skill".into());
        let (name, description, _body) = parse_skill_md(&content, &fallback)?;
        let name = valid_skill_name(&name)?;
        let target_dir = global_dir.join("skills").join(name);
        if target_dir.exists() {
            return Err(format!("技能 {name} 已存在，请先删除再导入"));
        }
        // Only clone/extend the global config directory; never scan outside it.
        let global_canonical = global_dir
            .canonicalize()
            .map_err(|_| "全局技能目录不可用".to_string())?;
        if skill_md
            .canonicalize()
            .map_err(|_| "无法解析导入路径".to_string())?
            .starts_with(&global_canonical)
        {
            return Err("该技能已位于全局目录中".into());
        }
        fs::create_dir_all(&target_dir).map_err(|_| "无法创建技能目录".to_string())?;
        fs::copy(skill_md, target_dir.join("SKILL.md"))
            .map_err(|_| "无法复制 SKILL.md".to_string())?;
        Ok(SkillView {
            name: name.to_string(),
            description,
            source: SkillSource::Global,
            path: target_dir
                .join("SKILL.md")
                .to_string_lossy()
                .into_owned(),
        })
    }

    /// Clone a repository shallowly, then import every SKILL.md found inside.
    pub async fn import_from_github(
        &self,
        global_dir: &Path,
        repo_url: &str,
    ) -> AppResult<Vec<SkillView>> {
        let url = repo_url.trim();
        let parsed = reqwest::Url::parse(url).map_err(|_| "仓库地址格式无效".to_string())?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("仅支持 https:// 仓库地址（不含账号、参数或片段）".into());
        }
        let temp = tempfile::tempdir().map_err(|_| "无法创建临时目录".to_string())?;
        let clone_target = temp.path().join("repo");
        let output = tokio::time::timeout(
            CLONE_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["clone", "--depth", "1", "--quiet", url])
                .arg(&clone_target)
                .output(),
        )
        .await
        .map_err(|_| "git clone 超时（60s）".to_string())?
        .map_err(|_| "无法执行 git，请确认已安装 Git".to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git clone 失败：{}",
                stderr.lines().next().unwrap_or("未知错误")
            ));
        }
        let mut imported = Vec::new();
        let mut stack = vec![clone_target];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if entry.file_name() != ".git" {
                        stack.push(path);
                    }
                } else if entry.file_name() == "SKILL.md" {
                    if let Ok(view) = self.import_skill_md_file(global_dir, &path) {
                        imported.push(view);
                    }
                }
            }
        }
        if imported.is_empty() {
            return Err("仓库中未找到可导入的 SKILL.md".into());
        }
        imported.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(imported)
    }

    fn import_skill_md_file(&self, global_dir: &Path, skill_md: &Path) -> AppResult<SkillView> {
        let content = fs::read_to_string(skill_md).map_err(|_| "无法读取 SKILL.md".to_string())?;
        let fallback = skill_md
            .parent()
            .and_then(|dir| dir.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "imported-skill".into());
        let (name, description, _body) = parse_skill_md(&content, &fallback)?;
        let name = valid_skill_name(&name)?;
        let target_dir = global_dir.join("skills").join(name);
        if target_dir.exists() {
            return Err(format!("技能 {name} 已存在"));
        }
        fs::create_dir_all(&target_dir).map_err(|_| "无法创建技能目录".to_string())?;
        fs::copy(skill_md, target_dir.join("SKILL.md"))
            .map_err(|_| "无法复制 SKILL.md".to_string())?;
        Ok(SkillView {
            name: name.to_string(),
            description,
            source: SkillSource::Global,
            path: target_dir
                .join("SKILL.md")
                .to_string_lossy()
                .into_owned(),
        })
    }
}

pub fn recommended_skills() -> Vec<RecommendedItem> {
    vec![
        RecommendedItem {
            id: "superpowers".into(),
            kind: "skill".into(),
            name: "Superpowers".into(),
            description: "社区热门技能合集：测试驱动开发、系统化调试、分支工作流等可复用工作流。".into(),
            source_url: "https://github.com/obra/superpowers".into(),
            install_payload: "https://github.com/obra/superpowers".into(),
        },
        RecommendedItem {
            id: "matt-pocock-skills".into(),
            kind: "skill".into(),
            name: "Matt Pocock Skills".into(),
            description: "TypeScript 与全栈工程实践技能集，来自 Total TypeScript 作者。".into(),
            source_url: "https://github.com/mattpocock/skills".into(),
            install_payload: "https://github.com/mattpocock/skills".into(),
        },
        RecommendedItem {
            id: "anthropic-skills".into(),
            kind: "skill".into(),
            name: "Anthropic 官方示例".into(),
            description: "Anthropic 发布的官方技能示例库，包含文档处理、代码审查等示范实现。".into(),
            source_url: "https://github.com/anthropics/skills".into(),
            install_payload: "https://github.com/anthropics/skills".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let content = "---\nname: pdf-tools\ndescription: Handle PDF files\n---\n\n# PDF\n\nDo the thing.\n";
        let (name, description, body) = parse_skill_md(content, "fallback").unwrap();
        assert_eq!(name, "pdf-tools");
        assert_eq!(description, "Handle PDF files");
        assert_eq!(body, "# PDF\n\nDo the thing.");
    }

    #[test]
    fn falls_back_to_directory_name_without_frontmatter() {
        let (name, description, body) = parse_skill_md("# Plain\n\nbody text", "my-dir").unwrap();
        assert_eq!(name, "my-dir");
        assert_eq!(description, "");
        assert_eq!(body, "# Plain\n\nbody text");
    }

    #[test]
    fn rejects_invalid_names_and_oversize() {
        for bad in ["", "Bad Name", "a--b", "-lead", "trail-", &"x".repeat(65)] {
            assert!(valid_skill_name(bad).is_err());
        }
        assert!(parse_skill_md(&format!("---\nname: ok\n---\n\n{}", "x".repeat(131073)), "ok").is_err());
    }

    #[test]
    fn project_skills_shadow_global() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let workspace = base.path().join("ws");
        fs::create_dir_all(global.join("skills").join("dup")).unwrap();
        fs::create_dir_all(workspace.join(".maestro").join("skills").join("dup")).unwrap();
        fs::write(global.join("skills").join("dup").join("SKILL.md"), "---\nname: dup\ndescription: global\n---\n\nG").unwrap();
        fs::write(workspace.join(".maestro").join("skills").join("dup").join("SKILL.md"), "---\nname: dup\ndescription: project\n---\n\nP").unwrap();
        let store = SkillStore::default();
        let views = store.scan(&global, Some(&workspace));
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].description, "project");
        assert_eq!(views[0].source, SkillSource::Project);
        assert_eq!(store.body("dup", &global, Some(&workspace)).unwrap(), "P");
    }

    #[test]
    fn create_and_import_skill_roundtrip() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let store = SkillStore::default();
        let view = store.create_new(&global, "my-skill", "desc").unwrap();
        assert_eq!(view.name, "my-skill");
        assert!(store.create_new(&global, "my-skill", "x").is_err());
        let external = base.path().join("external");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("SKILL.md"), "---\nname: imported\ndescription: ext\n---\n\nB").unwrap();
        let outcomes = store.import_from_path(&global, external.to_str().unwrap()).unwrap();
        assert_eq!(outcomes[0].name, "imported");
        assert_eq!(outcomes[0].status, "imported");
        assert!(store.import_from_path(&global, external.to_str().unwrap()).is_err());
    }

    #[test]
    fn import_parent_dir_batches_children() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let store = SkillStore::default();
        let parent = base.path().join("skills-pack");
        for (dir, name, desc) in [("alpha", "alpha-skill", "a"), ("beta", "beta-skill", "b")] {
            fs::create_dir_all(parent.join(dir)).unwrap();
            fs::write(parent.join(dir).join("SKILL.md"), format!("---\nname: {name}\ndescription: {desc}\n---\n\nB")).unwrap();
        }
        // An existing skill is skipped; a malformed child fails without stopping the rest.
        fs::create_dir_all(global.join("skills").join("alpha-skill")).unwrap();
        fs::write(global.join("skills").join("alpha-skill").join("SKILL.md"), "---\nname: alpha-skill\n---\n\nG").unwrap();
        fs::create_dir_all(parent.join("broken")).unwrap();
        fs::write(parent.join("broken").join("SKILL.md"), "---\nname: Bad Name!\n---\n\nB").unwrap();
        fs::create_dir_all(parent.join("not-a-skill")).unwrap();

        let outcomes = store.import_from_path(&global, parent.to_str().unwrap()).unwrap();
        let by_status = |status: &str| outcomes.iter().filter(|o| o.status == status).count();
        assert_eq!(by_status("imported"), 1, "{outcomes:?}");
        assert_eq!(by_status("skipped"), 1, "{outcomes:?}");
        assert_eq!(by_status("failed"), 1, "{outcomes:?}");
        assert!(global.join("skills").join("beta-skill").join("SKILL.md").is_file());
    }
}
