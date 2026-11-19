//! 声明式插件清单：仓库没有独立插件运行时，「插件」是
//! `~/.maestro/plugins/<name>.json` 与 `<workspace>/.maestro/plugins/<name>.json`
//! 下的 JSON 清单，把一个名字别名到底层 Skill 或 MCP 依赖上。
//! 架构沙盘的插件节点按清单解析出真实依赖再强制调用；清单本身不可执行。
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::adapters::skills::valid_skill_name;
use crate::domain::{
    AppResult, PluginCondition, PluginKind, PluginManifest, PluginView, SkillSource,
};

const MAX_MANIFEST_BYTES: usize = 65_536;
const MAX_PLUGINS_PER_ROOT: usize = 128;

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub source: SkillSource,
}

impl PluginEntry {
    fn view(&self) -> PluginView {
        let manifest = &self.manifest;
        PluginView {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            kind: Some(manifest.kind),
            target: Some(manifest.target.clone()),
            tool: manifest.tool.clone(),
            condition: manifest.condition,
            params: manifest.params.clone(),
            source: self.source,
            path: self.path.to_string_lossy().into_owned(),
            error: None,
        }
    }
}

fn error_view(path: &Path, source: SkillSource, reason: &str) -> PluginView {
    PluginView {
        name: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into()),
        description: String::new(),
        kind: None,
        target: None,
        tool: None,
        condition: PluginCondition::Required,
        params: BTreeMap::new(),
        source,
        path: path.to_string_lossy().into_owned(),
        error: Some(reason.to_string()),
    }
}

fn parse_manifest(content: &str, fallback_name: &str) -> AppResult<PluginManifest> {
    if content.len() > MAX_MANIFEST_BYTES {
        return Err("插件清单超过 64KB 限制".into());
    }
    let mut manifest: PluginManifest = serde_json::from_str(content)
        .map_err(|error| format!("插件清单 JSON 无法解析：{error}"))?;
    if manifest.name.trim().is_empty() {
        manifest.name = fallback_name.to_string();
    }
    manifest.name = valid_skill_name(&manifest.name)
        .map_err(|_| {
            format!("插件名 {} 必须为 kebab-case（小写字母、数字、连字符）", manifest.name)
        })?
        .to_string();
    let target = manifest.target.trim();
    if target.is_empty() || target.chars().count() > 128 {
        return Err(format!("插件 {} 的 target 缺失或超长", manifest.name));
    }
    manifest.target = target.to_string();
    match manifest.kind {
        PluginKind::Skill => {
            if manifest.tool.is_some() {
                return Err(format!("插件 {}（skill）不允许 tool 字段", manifest.name));
            }
        }
        PluginKind::Mcp => {
            if let Some(tool) = manifest.tool.as_deref() {
                if tool.trim().is_empty() || tool.chars().count() > 128 {
                    return Err(format!("插件 {} 的 tool 非法", manifest.name));
                }
            }
        }
    }
    if manifest.description.chars().count() > 512 {
        manifest.description = manifest.description.chars().take(512).collect();
    }
    Ok(manifest)
}

fn scan_root(
    dir: &Path,
    source: SkillSource,
    into: &mut BTreeMap<String, PluginEntry>,
    errors: &mut Vec<PluginView>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten().take(MAX_PLUGINS_PER_ROOT) {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            || path.extension().map(|ext| ext != "json").unwrap_or(true)
        {
            continue;
        }
        let fallback = entry
            .file_name()
            .to_string_lossy()
            .trim_end_matches(".json")
            .to_string();
        let scanned = fs::read_to_string(&path)
            .map_err(|_| "无法读取插件清单".to_string())
            .and_then(|content| parse_manifest(&content, &fallback));
        match scanned {
            Ok(manifest) => {
                into.insert(
                    manifest.name.clone(),
                    PluginEntry { manifest, path: path.clone(), source },
                );
            }
            Err(reason) => errors.push(error_view(&path, source, &reason)),
        }
    }
}

/// 项目清单覆盖全局同名清单（与 Skill 作用域规则一致）。
#[derive(Default)]
pub struct PluginStore {
    plugins: Mutex<BTreeMap<String, PluginEntry>>,
}

impl PluginStore {
    pub fn scan(&self, global_dir: &Path, workspace_dir: Option<&Path>) -> Vec<PluginView> {
        let mut found = BTreeMap::new();
        let mut errors = Vec::new();
        scan_root(&global_dir.join("plugins"), SkillSource::Global, &mut found, &mut errors);
        if let Some(workspace) = workspace_dir {
            scan_root(
                &workspace.join(".maestro").join("plugins"),
                SkillSource::Project,
                &mut found,
                &mut errors,
            );
        }
        *self.plugins.lock().unwrap() = found.clone();
        let mut views: Vec<_> = found.values().map(PluginEntry::view).collect();
        views.extend(errors);
        views.sort_by(|a, b| a.name.cmp(&b.name));
        views
    }

    /// 清单名 → 解析后清单（供架构求值快照使用）。
    pub fn manifests(
        &self,
        global_dir: &Path,
        workspace_dir: Option<&Path>,
    ) -> BTreeMap<String, PluginManifest> {
        self.scan(global_dir, workspace_dir);
        self.plugins
            .lock()
            .unwrap()
            .iter()
            .map(|(name, entry)| (name.clone(), entry.manifest.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_manifest_and_rejects_skill_tool() {
        let ok = parse_manifest(
            r#"{"name":"pg-check","kind":"mcp","target":"pg","tool":"query","description":"d"}"#,
            "x",
        )
        .unwrap();
        assert_eq!(ok.name, "pg-check");
        assert_eq!(ok.tool.as_deref(), Some("query"));
        assert_eq!(ok.condition, PluginCondition::Required);

        let bad = parse_manifest(r#"{"name":"s","kind":"skill","target":"doc","tool":"t"}"#, "x");
        assert!(bad.is_err());
    }

    #[test]
    fn project_manifest_shadows_global() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let ws = base.path().join("ws");
        fs::create_dir_all(global.join("plugins")).unwrap();
        fs::create_dir_all(ws.join(".maestro").join("plugins")).unwrap();
        fs::write(
            global.join("plugins").join("p.json"),
            r#"{"name":"p","kind":"skill","target":"global-skill"}"#,
        )
        .unwrap();
        fs::write(
            ws.join(".maestro").join("plugins").join("p.json"),
            r#"{"name":"p","kind":"skill","target":"project-skill"}"#,
        )
        .unwrap();
        let store = PluginStore::default();
        let views = store.scan(&global, Some(&ws));
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].target.as_deref(), Some("project-skill"));
        assert_eq!(views[0].source, SkillSource::Project);
    }

    #[test]
    fn invalid_manifest_surfaces_as_error_view() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        fs::create_dir_all(global.join("plugins")).unwrap();
        fs::write(global.join("plugins").join("broken.json"), "not-json").unwrap();
        let store = PluginStore::default();
        let views = store.scan(&global, None);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "broken");
        assert!(views[0].error.is_some());
        assert!(views[0].kind.is_none());
    }
}
