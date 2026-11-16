use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
use crate::domain::{AgentDefinition, AgentSource, AppResult};

/// Built-ins are always available and can be overridden by a same-named file.
const BUILTIN_NAMES: [&str; 2] = ["explore", "general"];

fn fallback(name: &str, source: AgentSource) -> AgentDefinition {
    AgentDefinition {
        name: name.into(), description: String::new(), category: None, mode: "subagent".into(),
        model: None, prompt: String::new(), tools: Vec::new(), steps: None, hidden: false, color: None, source,
    }
}

fn parse(content: &str, fallback_name: &str, source: AgentSource) -> AgentDefinition {
    let mut agent = AgentDefinition::builtin(fallback_name).unwrap_or_else(|| fallback(fallback_name, source));
    agent.source = source;
    let mut body = content;
    if let Some(rest) = content.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                let Some((key, value)) = line.split_once(':') else { continue };
                let value = value.trim().trim_matches('"').trim_matches('\'');
                match key.trim() {
                    "name" => agent.name = value.into(),
                    "description" => agent.description = value.into(),
                    "category" => agent.category = Some(value.into()),
                    "mode" => agent.mode = value.into(),
                    "model" => if !value.is_empty() { agent.model = Some(value.into()) },
                    "tools" => agent.tools = value.split(',').map(|item| item.trim().to_string()).filter(|item| !item.is_empty()).collect(),
                    "steps" => agent.steps = value.parse().ok(),
                    "hidden" => agent.hidden = value == "true",
                    "color" => agent.color = Some(value.into()),
                    _ => {}
                }
            }
            body = rest[end + 4..].trim_start_matches('\n');
        }
    }
    agent.prompt = body.trim().to_string();
    agent
}

fn read_dir(dir: &Path, source: AgentSource) -> Vec<AgentDefinition> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries.filter_map(Result::ok).filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok().map(|content| parse(&content, &entry.path().file_stem().unwrap_or_default().to_string_lossy(), source)))
        .collect()
}

pub fn global_dir(maestro: &Path) -> PathBuf { maestro.join("agents") }
pub fn project_dir(workspace: &Path) -> PathBuf { workspace.join(".maestro").join("agents") }

/// Resolved runtime list: builtin < global < project, sorted by name. A file in
/// a higher source replaces a same-named definition from a lower one.
pub fn effective(maestro: &Path, workspace: Option<&Path>) -> Vec<AgentDefinition> {
    let mut merged: BTreeMap<String, AgentDefinition> = BTreeMap::new();
    for name in BUILTIN_NAMES {
        if let Some(agent) = AgentDefinition::builtin(name) {
            merged.insert(agent.name.clone(), agent);
        }
    }
    for agent in read_dir(&global_dir(maestro), AgentSource::Global) {
        merged.insert(agent.name.clone(), agent);
    }
    if let Some(root) = workspace {
        for agent in read_dir(&project_dir(root), AgentSource::Project) {
            merged.insert(agent.name.clone(), agent);
        }
    }
    merged.into_values().collect()
}

/// `scope` selects a single on-disk source; `all` is the merged runtime view
/// (builtins included) the UI and `spawn_subagent` both use.
pub fn list(maestro: &Path, workspace: Option<&Path>, scope: &str) -> Vec<AgentDefinition> {
    match scope {
        "all" => effective(maestro, workspace),
        "project" => workspace.map(|path| read_dir(&project_dir(path), AgentSource::Project)).unwrap_or_default(),
        _ => read_dir(&global_dir(maestro), AgentSource::Global),
    }
}

pub fn save(maestro: &Path, workspace: Option<&Path>, scope: &str, agent: &AgentDefinition) -> AppResult<()> {
    if agent.name.trim().is_empty() || agent.name.len() > 64 || !agent.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Agent 名称必须为 1-64 位字母/数字/-/_".into());
    }
    let dir = if scope == "project" { project_dir(workspace.ok_or("未选择工作区")?) } else { global_dir(maestro) };
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let tools = agent.tools.join(", ");
    let content = format!("---\nname: {}\ndescription: {}\ncategory: {}\nmode: {}\nmodel: {}\ntools: {}\nsteps: {}\nhidden: {}\ncolor: {}\n---\n\n{}\n",
        agent.name, agent.description, agent.category.clone().unwrap_or_default(), agent.mode,
        agent.model.clone().unwrap_or_default(), tools, agent.steps.map(|v| v.to_string()).unwrap_or_default(),
        agent.hidden, agent.color.clone().unwrap_or_default(), agent.prompt);
    fs::write(dir.join(format!("{}.md", agent.name)), content).map_err(|error| error.to_string())
}

pub fn delete(maestro: &Path, workspace: Option<&Path>, scope: &str, name: &str) -> AppResult<()> {
    if name.contains(['/', '\\', '.']) { return Err("无效 Agent 名称".into()); }
    let dir = if scope == "project" { project_dir(workspace.ok_or("未选择工作区")?) } else { global_dir(maestro) };
    let path = dir.join(format!("{name}.md"));
    if path.exists() { fs::remove_file(path).map_err(|error| error.to_string()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, description: &str, source: AgentSource) -> AgentDefinition {
        AgentDefinition {
            name: name.into(), description: description.into(), category: Some("研究".into()),
            mode: "subagent".into(), model: None, prompt: "调研".into(),
            tools: vec!["read_file".into(), "list_dir".into()], steps: Some(4),
            hidden: false, color: None, source,
        }
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let root = tempfile::tempdir().unwrap();
        let dir = global_dir(root.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("reviewer.md"), "---\nname: reviewer\ndescription: 代码评审\ncategory: 质量\nmode: subagent\nmodel: gpt-x\ntools: read_file, list_dir\nsteps: 6\nhidden: true\ncolor: #fff\n---\n\n只读评审。\n").unwrap();

        let agents = list(root.path(), None, "global");
        assert_eq!(agents.len(), 1);
        let parsed = &agents[0];
        assert_eq!(parsed.name, "reviewer");
        assert_eq!(parsed.description, "代码评审");
        assert_eq!(parsed.category.as_deref(), Some("质量"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-x"));
        assert_eq!(parsed.tools, vec!["read_file", "list_dir"]);
        assert_eq!(parsed.steps, Some(6));
        assert!(parsed.hidden);
        assert_eq!(parsed.prompt, "只读评审。");
        assert_eq!(parsed.source, AgentSource::Global);
    }

    #[test]
    fn all_scope_prefers_project_over_global_over_builtin() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(global_dir(root.path())).unwrap();
        fs::create_dir_all(project_dir(workspace.path())).unwrap();
        save(root.path(), None, "global", &agent("explore", "global override", AgentSource::Global)).unwrap();
        save(root.path(), Some(workspace.path()), "project", &agent("explore", "project override", AgentSource::Project)).unwrap();
        save(root.path(), None, "global", &agent("reviewer", "global only", AgentSource::Global)).unwrap();

        let agents = list(root.path(), Some(workspace.path()), "all");
        let names: Vec<&str> = agents.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["explore", "general", "reviewer"]);
        assert_eq!(agents[0].description, "project override");
        assert_eq!(agents[0].source, AgentSource::Project);
        assert_eq!(agents[1].source, AgentSource::Builtin);
        assert_eq!(agents[2].source, AgentSource::Global);
    }

    #[test]
    fn save_roundtrips_and_ignores_source_in_frontmatter() {
        let root = tempfile::tempdir().unwrap();
        save(root.path(), None, "global", &agent("reviewer", "代码评审", AgentSource::Builtin)).unwrap();
        let text = fs::read_to_string(global_dir(root.path()).join("reviewer.md")).unwrap();
        assert!(!text.contains("source:"));
        let agents = list(root.path(), None, "global");
        assert_eq!(agents[0].name, "reviewer");
        assert_eq!(agents[0].source, AgentSource::Global);
        assert_eq!(agents[0].tools, vec!["read_file", "list_dir"]);
    }

    #[test]
    fn rejects_unsafe_names() {
        let root = tempfile::tempdir().unwrap();
        assert!(save(root.path(), None, "global", &agent("../evil", "x", AgentSource::Global)).is_err());
        assert!(delete(root.path(), None, "global", "../evil").is_err());
    }

    #[test]
    fn delete_only_touches_the_requested_scope() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        save(root.path(), None, "global", &agent("reviewer", "global", AgentSource::Global)).unwrap();
        save(root.path(), Some(workspace.path()), "project", &agent("reviewer", "project", AgentSource::Project)).unwrap();

        delete(root.path(), None, "global", "reviewer").unwrap();
        assert!(list(root.path(), None, "global").is_empty());
        let project = list(root.path(), Some(workspace.path()), "project");
        assert_eq!(project.len(), 1);
        assert_eq!(project[0].description, "project");
    }
}
