use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
use crate::domain::{AgentDefinition, AgentImportOutcome, AgentItemRef, AgentSource, AppResult, SubagentDefaults, MAX_SUBAGENT_TOOL_BUDGET};

/// Built-ins are always available and can be overridden by a same-named file.
const BUILTIN_NAMES: [&str; 2] = ["explore", "general"];

fn fallback(name: &str, source: AgentSource) -> AgentDefinition {
    AgentDefinition {
        name: name.into(), description: String::new(), category: None, mode: "subagent".into(),
        model: None, prompt: String::new(), tools: Vec::new(), steps: None, tool_budget: None, hidden: false, color: None, source,
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
                    "tool_budget" => agent.tool_budget = value.parse().ok(),
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

/// Whether the workspace carries project `.maestro/` assets that would be
/// loaded at runtime. Any non-empty agents/skills/commands directory counts —
/// an empty `.maestro` folder alone does not require trust.
pub fn has_maestro_assets(workspace: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    for dir in ["agents", "skills", "commands"] {
        let present = workspace
            .join(".maestro")
            .join(dir)
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        if present {
            found.push(dir);
        }
    }
    found
}

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
    let content = format!("---\nname: {}\ndescription: {}\ncategory: {}\nmode: {}\nmodel: {}\ntools: {}\nsteps: {}\ntool_budget: {}\nhidden: {}\ncolor: {}\n---\n\n{}\n",
        agent.name, agent.description, agent.category.clone().unwrap_or_default(), agent.mode,
        agent.model.clone().unwrap_or_default(), tools, agent.steps.map(|v| v.to_string()).unwrap_or_default(),
        agent.tool_budget.map(|v| v.to_string()).unwrap_or_default(),
        agent.hidden, agent.color.clone().unwrap_or_default(), agent.prompt);
    fs::write(dir.join(format!("{}.md", agent.name)), content).map_err(|error| error.to_string())
}

pub fn delete(maestro: &Path, workspace: Option<&Path>, scope: &str, name: &str) -> AppResult<()> {
    if name.contains(['/', '\\', '.']) { return Err("无效 Agent 名称".into()); }
    let dir = if scope == "project" { project_dir(workspace.ok_or("未选择工作区")?) } else { global_dir(maestro) };
    let path = dir.join(format!("{name}.md"));
    if path.exists() { fs::remove_file(path).map_err(|error| error.to_string()) } else { Ok(()) }
}

/// 批量启用/禁用：`hidden` 即禁用语义（spawn 枚举、具名派发、物料均过滤）。
/// 逐项三态返回，单项失败不阻断其余。条目按 effective() 里 def.source 定位——
/// builtin 禁用写一份 global override 存根（builtin def + hidden:true），
/// 启用时 override 与 builtin 等价（除 hidden/source）则删除存根恢复 builtin，
/// 不等价（用户自定义过字段）只把 override.hidden 翻回 false。
pub fn set_hidden(
    maestro: &Path, workspace: Option<&Path>,
    items: &[AgentItemRef], hidden: bool,
) -> Vec<AgentImportOutcome> {
    let defs = effective(maestro, workspace);
    items.iter().map(|item| {
        match set_one_hidden(maestro, workspace, &defs, &item.name, hidden) {
            Ok(changed) => AgentImportOutcome {
                name: item.name.clone(), status: if changed { "updated".into() } else { "skipped".into() }, error: None,
            },
            Err(error) => AgentImportOutcome { name: item.name.clone(), status: "failed".into(), error: Some(error) },
        }
    }).collect()
}

fn set_one_hidden(
    maestro: &Path, workspace: Option<&Path>,
    defs: &[AgentDefinition], name: &str, hidden: bool,
) -> AppResult<bool> {
    let def = defs.iter().find(|agent| agent.name == name)
        .ok_or_else(|| format!("子智能体 {name} 不存在"))?;
    match def.source {
        AgentSource::Builtin if hidden => {
            let mut stub = def.clone();
            stub.hidden = true;
            save(maestro, workspace, "global", &stub)?;
            Ok(true)
        }
        AgentSource::Builtin => restore_builtin(maestro, def),
        // 启用 global override：同名 builtin 存在时走等价检测——纯存根删文件
        // 恢复 builtin 徽标；用户自定义过的 override 只翻 hidden 保留。
        AgentSource::Global if !hidden => {
            if let Some(builtin) = AgentDefinition::builtin(&def.name) {
                restore_builtin(maestro, &builtin)
            } else {
                flip_hidden(maestro, workspace, def, false)
            }
        }
        _ => flip_hidden(maestro, workspace, def, hidden),
    }
}

fn flip_hidden(
    maestro: &Path, workspace: Option<&Path>, def: &AgentDefinition, hidden: bool,
) -> AppResult<bool> {
    if def.hidden == hidden { return Ok(false); }
    let mut next = def.clone();
    next.hidden = hidden;
    let scope = if def.source == AgentSource::Project { "project" } else { "global" };
    save(maestro, workspace, scope, &next)?;
    Ok(true)
}

fn restore_builtin(maestro: &Path, builtin: &AgentDefinition) -> AppResult<bool> {
    let path = global_dir(maestro).join(format!("{}.md", builtin.name));
    if !path.exists() { return Ok(false); }
    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let parsed = parse(&content, &builtin.name, AgentSource::Global);
    if same_except_hidden(&parsed, builtin) {
        fs::remove_file(&path).map_err(|error| error.to_string())?;
    } else {
        let mut next = parsed;
        next.hidden = false;
        save(maestro, None, "global", &next)?;
    }
    Ok(true)
}

fn same_except_hidden(a: &AgentDefinition, b: &AgentDefinition) -> bool {
    a.name == b.name && a.description == b.description && a.category == b.category
        && a.mode == b.mode && a.model == b.model && a.prompt == b.prompt
        && a.tools == b.tools && a.steps == b.steps
        && a.tool_budget == b.tool_budget && a.color == b.color
}

/// 批量删除：逐项三态；builtin 直接记 failed（无文件可删）。
pub fn delete_many(
    maestro: &Path, workspace: Option<&Path>, items: &[AgentItemRef],
) -> Vec<AgentImportOutcome> {
    items.iter().map(|item| {
        let result = if item.scope == "builtin" {
            Err("内置子智能体不可删除".into())
        } else {
            delete(maestro, workspace, &item.scope, &item.name)
        };
        match result {
            Ok(()) => AgentImportOutcome { name: item.name.clone(), status: "deleted".into(), error: None },
            Err(error) => AgentImportOutcome { name: item.name.clone(), status: "failed".into(), error: Some(error) },
        }
    }).collect()
}

// ---- 全局默认（~/.maestro/agents.json） ------------------------------------

pub fn defaults_path(maestro: &Path) -> PathBuf { maestro.join("agents.json") }

/// Missing file → all-`None` defaults (i.e. the pre-feature behavior);
/// malformed JSON is an error — a broken config must surface, not silently reset.
pub fn load_defaults(maestro: &Path) -> AppResult<SubagentDefaults> {
    match fs::read_to_string(defaults_path(maestro)) {
        Ok(content) => serde_json::from_str(&content).map_err(|error| format!("agents.json 解析失败：{error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SubagentDefaults::default()),
        Err(error) => Err(error.to_string()),
    }
}

fn validate_defaults(defaults: &SubagentDefaults) -> AppResult<()> {
    if let Some(steps) = defaults.steps {
        if !(1..=64).contains(&steps) { return Err("默认轮次须在 1-64 之间".into()); }
    }
    if let Some(budget) = defaults.tool_budget {
        if !(1..=MAX_SUBAGENT_TOOL_BUDGET).contains(&budget) { return Err(format!("默认预算须在 1-{MAX_SUBAGENT_TOOL_BUDGET} 之间")); }
    }
    if let Some(parallel) = defaults.max_parallel {
        if !(1..=4).contains(&parallel) { return Err("每轮并行上限须在 1-4 之间".into()); }
    }
    if let Some(total) = defaults.max_total {
        if !(1..=16).contains(&total) { return Err("单 run 总额度须在 1-16 之间".into()); }
    }
    Ok(())
}

pub fn save_defaults(maestro: &Path, defaults: &SubagentDefaults) -> AppResult<()> {
    validate_defaults(defaults)?;
    fs::create_dir_all(maestro).map_err(|error| error.to_string())?;
    let content = serde_json::to_string_pretty(defaults).map_err(|error| error.to_string())?;
    fs::write(defaults_path(maestro), content).map_err(|error| error.to_string())
}

// ---- 批量导入（agency-agents-zh 类智能体仓库） ------------------------------

/// 文件 stem → Agent slug：仅保留 ASCII 字母/数字/-/_，其余折叠为 '-'。
/// agency-agents-zh 一类仓库用 ASCII 文件名承载 slug、frontmatter `name`
/// 是中文显示名——slug 永远取自文件名，不读 frontmatter。
fn sanitize_agent_name(raw: &str) -> Option<String> {
    let mut slug = String::with_capacity(raw.len());
    let mut last_dash = true; // 开头非字母数字不产生前导 '-'
    for ch in raw.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    (1..=64).contains(&slug.len()).then_some(slug)
}

/// 源文件 → 本仓 Agent 定义：slug 来自文件名；frontmatter 中文 `name` 回退
/// 为 description（parse 已把它放进 agent.name）；category 缺省取父目录名；
/// 强制 `mode: subagent`——导入语义即"作为子智能体装配"。
fn imported_agent(content: &str, slug: &str, fallback_category: Option<&str>) -> AgentDefinition {
    let mut agent = parse(content, slug, AgentSource::Global);
    let display_name = agent.name.clone();
    agent.name = slug.to_string();
    if agent.description.trim().is_empty() && display_name != slug {
        agent.description = display_name;
    }
    if agent.category.as_deref().map(str::trim).unwrap_or_default().is_empty() {
        agent.category = fallback_category.map(str::to_string);
    }
    agent.mode = "subagent".into();
    agent
}

/// 仓库说明/治理文件不是智能体定义——批量导入时跳过。
fn is_agent_markdown(name: &str) -> bool {
    let lower = name.to_lowercase();
    let Some(stem) = lower.strip_suffix(".md") else {
        return false;
    };
    !matches!(stem,
        "readme" | "contributing" | "changelog" | "changes" | "license" | "licence"
        | "security" | "code_of_conduct" | "agents" | "claude" | "gemini" | "notice")
}

/// 批量导入目录中的 `*.md` 为全局 Agent：递归扫描、跳过隐藏/VCS 目录与
/// 说明文档；同名文件已存在则 skipped（不静默覆盖手工改动，与 skills 一致）。
pub fn import_from_dir(maestro: &Path, source_path: &str) -> AppResult<Vec<AgentImportOutcome>> {
    let source = Path::new(source_path)
        .canonicalize()
        .map_err(|_| "导入路径不存在".to_string())?;
    if !source.is_dir() {
        return Err("导入路径必须是包含 .md 智能体文件的目录".into());
    }
    // 防止把已装 Agent 目录当作来源再"导入"一遍。
    let maestro_canonical = maestro.canonicalize().unwrap_or_else(|_| maestro.to_path_buf());
    if source.starts_with(&maestro_canonical) {
        return Err("不能从 ~/.maestro 目录自身导入".into());
    }
    let target_root = global_dir(maestro);
    fs::create_dir_all(&target_root).map_err(|error| error.to_string())?;

    let mut files = Vec::new();
    let mut stack = vec![source.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && name != "target" {
                    stack.push(path);
                }
            } else if is_agent_markdown(&name) {
                files.push(path);
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Err("目录中未找到可导入的 .md 智能体文件".into());
    }

    let mut outcomes = Vec::new();
    for file in files {
        let label = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let Some(slug) = sanitize_agent_name(&label) else {
            outcomes.push(AgentImportOutcome { name: label, status: "failed".into(),
                error: Some("文件名无法转换为合法 Agent slug（需含 ASCII 字母/数字）".into()) });
            continue;
        };
        if target_root.join(format!("{slug}.md")).exists() {
            outcomes.push(AgentImportOutcome { name: slug, status: "skipped".into(),
                error: Some("同名 Agent 已存在，如需更新请先删除".into()) });
            continue;
        }
        let category = file
            .parent()
            .filter(|parent| *parent != source)
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let result = fs::read_to_string(&file)
            .map_err(|error| error.to_string())
            .and_then(|content| {
                save(maestro, None, "global", &imported_agent(&content, &slug, category.as_deref()))
            });
        outcomes.push(match result {
            Ok(()) => AgentImportOutcome { name: slug, status: "imported".into(), error: None },
            Err(error) => AgentImportOutcome { name: slug, status: "failed".into(), error: Some(error) },
        });
    }
    outcomes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(outcomes)
}

const AGENT_CLONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 浅克隆仓库后按目录导入——agency-agents-zh 类智能体包的一键安装路径。
pub async fn import_from_github(maestro: &Path, repo_url: &str) -> AppResult<Vec<AgentImportOutcome>> {
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
        AGENT_CLONE_TIMEOUT,
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
        return Err(format!("git clone 失败：{}", stderr.lines().next().unwrap_or("未知错误")));
    }
    import_from_dir(maestro, &clone_target.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, description: &str, source: AgentSource) -> AgentDefinition {
        AgentDefinition {
            name: name.into(), description: description.into(), category: Some("研究".into()),
            mode: "subagent".into(), model: None, prompt: "调研".into(),
            tools: vec!["read_file".into(), "list_dir".into()], steps: Some(4),
            tool_budget: None, hidden: false, color: None, source,
        }
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let root = tempfile::tempdir().unwrap();
        let dir = global_dir(root.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("reviewer.md"), "---\nname: reviewer\ndescription: 代码评审\ncategory: 质量\nmode: subagent\nmodel: gpt-x\ntools: read_file, list_dir\nsteps: 6\ntool_budget: 60\nhidden: true\ncolor: #fff\n---\n\n只读评审。\n").unwrap();

        let agents = list(root.path(), None, "global");
        assert_eq!(agents.len(), 1);
        let parsed = &agents[0];
        assert_eq!(parsed.name, "reviewer");
        assert_eq!(parsed.description, "代码评审");
        assert_eq!(parsed.category.as_deref(), Some("质量"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-x"));
        assert_eq!(parsed.tools, vec!["read_file", "list_dir"]);
        assert_eq!(parsed.steps, Some(6));
        assert_eq!(parsed.tool_budget, Some(60));
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
    fn tool_budget_roundtrips_and_defaults_to_none() {
        let root = tempfile::tempdir().unwrap();
        let mut budgeted = agent("budgeted", "x", AgentSource::Global);
        budgeted.tool_budget = Some(150);
        save(root.path(), None, "global", &budgeted).unwrap();
        let text = fs::read_to_string(global_dir(root.path()).join("budgeted.md")).unwrap();
        assert!(text.contains("tool_budget: 150"));

        save(root.path(), None, "global", &agent("plain", "x", AgentSource::Global)).unwrap();
        let agents = list(root.path(), None, "global");
        assert_eq!(agents.iter().find(|item| item.name == "budgeted").unwrap().tool_budget, Some(150));
        assert_eq!(agents.iter().find(|item| item.name == "plain").unwrap().tool_budget, None);
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

    #[test]
    fn sanitize_agent_name_keeps_ascii_and_folds_rest() {
        assert_eq!(sanitize_agent_name("CTO-Advisor"), Some("cto-advisor".into()));
        assert_eq!(sanitize_agent_name("tech lead v2"), Some("tech-lead-v2".into()));
        assert_eq!(sanitize_agent_name("a..b"), Some("a-b".into()));
        assert_eq!(sanitize_agent_name("研发专家"), None, "纯非 ASCII 文件名无 slug 可派生");
        assert_eq!(sanitize_agent_name("-"), None);
    }

    #[test]
    fn imported_agent_maps_zh_name_to_description_and_forces_mode() {
        let content = "---\nname: 首席技术官\ntools: read_file\n---\n\n你是技术路线评审官。\n";
        let agent = imported_agent(content, "cto-advisor", Some("leadership"));
        assert_eq!(agent.name, "cto-advisor");
        assert_eq!(agent.description, "首席技术官", "中文显示名归位描述");
        assert_eq!(agent.mode, "subagent");
        assert_eq!(agent.category.as_deref(), Some("leadership"), "category 缺省回退父目录名");
        assert_eq!(agent.prompt, "你是技术路线评审官。");
    }

    #[test]
    fn import_from_dir_converts_pack_and_skips_duplicates() {
        let maestro = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("cto-advisor.md"), "---\nname: 首席技术官\n---\n\n技术评审。\n").unwrap();
        fs::create_dir_all(source.path().join("quality")).unwrap();
        fs::write(source.path().join("quality").join("reviewer.md"), "---\nname: 评审官\ndescription: 代码评审\n---\n\n评审。\n").unwrap();
        // 说明文档与隐藏目录不进入导入集。
        fs::write(source.path().join("README.md"), "# pack\n").unwrap();
        fs::create_dir_all(source.path().join(".git")).unwrap();
        fs::write(source.path().join(".git").join("agent.md"), "---\nname: x\n---\n").unwrap();
        fs::write(source.path().join("研发专家.md"), "---\nname: 研发专家\n---\n\n研发。\n").unwrap();

        let outcomes = import_from_dir(maestro.path(), source.path().to_str().unwrap()).unwrap();
        let status = |name: &str| outcomes.iter().find(|item| item.name == name).map(|item| item.status.as_str());
        assert_eq!(outcomes.len(), 3);
        assert_eq!(status("cto-advisor"), Some("imported"));
        assert_eq!(status("reviewer"), Some("imported"));
        assert_eq!(status("研发专家"), Some("failed"), "无 ASCII slug 的文件计为失败项");

        let agents = list(maestro.path(), None, "global");
        let cto = agents.iter().find(|item| item.name == "cto-advisor").unwrap();
        assert_eq!(cto.description, "首席技术官");
        assert_eq!(cto.mode, "subagent");
        let reviewer = agents.iter().find(|item| item.name == "reviewer").unwrap();
        assert_eq!(reviewer.description, "代码评审", "已有 description 不被中文名覆盖");
        assert_eq!(reviewer.category.as_deref(), Some("quality"), "category 缺省取父目录名");

        // 重复导入同名 → skipped，不覆盖。
        let again = import_from_dir(maestro.path(), source.path().to_str().unwrap()).unwrap();
        assert_eq!(again.iter().find(|item| item.name == "cto-advisor").unwrap().status, "skipped");
        assert!(agents.len() >= 2);
    }

    #[test]
    fn import_from_dir_rejects_maestro_dir_itself() {
        let maestro = tempfile::tempdir().unwrap();
        fs::create_dir_all(global_dir(maestro.path())).unwrap();
        assert!(import_from_dir(maestro.path(), maestro.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn defaults_missing_file_is_all_none_and_roundtrips() {
        let maestro = tempfile::tempdir().unwrap();
        assert_eq!(load_defaults(maestro.path()).unwrap(), SubagentDefaults::default());

        let defaults = SubagentDefaults {
            model: Some("gpt-sub".into()), steps: Some(12), tool_budget: Some(80),
            max_parallel: Some(4), max_total: Some(12),
        };
        save_defaults(maestro.path(), &defaults).unwrap();
        assert_eq!(load_defaults(maestro.path()).unwrap(), defaults);
        let raw = fs::read_to_string(defaults_path(maestro.path())).unwrap();
        assert!(raw.contains("\"toolBudget\": 80"), "camelCase 落盘：{raw}");
    }

    #[test]
    fn defaults_rejects_out_of_range_and_bad_json() {
        let maestro = tempfile::tempdir().unwrap();
        let too_many = SubagentDefaults { max_total: Some(99), ..Default::default() };
        assert!(save_defaults(maestro.path(), &too_many).is_err());
        let too_wide = SubagentDefaults { max_parallel: Some(9), ..Default::default() };
        assert!(save_defaults(maestro.path(), &too_wide).is_err());

        fs::write(defaults_path(maestro.path()), "{ not json").unwrap();
        assert!(load_defaults(maestro.path()).is_err(), "坏 JSON 必须报错而非静默重置");
    }

    fn item_ref(scope: &str, name: &str) -> AgentItemRef {
        AgentItemRef { scope: scope.into(), name: name.into() }
    }

    #[test]
    fn set_hidden_flips_global_and_skips_noop() {
        let maestro = tempfile::tempdir().unwrap();
        save(maestro.path(), None, "global", &agent("reviewer", "评审", AgentSource::Global)).unwrap();
        let out = set_hidden(maestro.path(), None, &[item_ref("global", "reviewer")], true);
        assert_eq!(out[0].status, "updated");
        let listed = list(maestro.path(), None, "all");
        assert!(listed.iter().find(|a| a.name == "reviewer").unwrap().hidden);

        let again = set_hidden(maestro.path(), None, &[item_ref("global", "reviewer")], true);
        assert_eq!(again[0].status, "skipped", "已是目标值不重复落盘");
        let missing = set_hidden(maestro.path(), None, &[item_ref("global", "ghost")], true);
        assert_eq!(missing[0].status, "failed");
    }

    #[test]
    fn set_hidden_builtin_disables_via_stub_and_restores() {
        let maestro = tempfile::tempdir().unwrap();
        let out = set_hidden(maestro.path(), None, &[item_ref("builtin", "explore")], true);
        assert_eq!(out[0].status, "updated");
        // 禁用后 effective 视图：explore 变成 hidden 的 global override。
        let def = list(maestro.path(), None, "all").into_iter().find(|a| a.name == "explore").unwrap();
        assert!(def.hidden && def.source == AgentSource::Global);

        // 启用：存根与 builtin 等价 → 删除文件，builtin 徽标与启用态恢复。
        let back = set_hidden(maestro.path(), None, &[item_ref("global", "explore")], false);
        assert_eq!(back[0].status, "updated");
        let def = list(maestro.path(), None, "all").into_iter().find(|a| a.name == "explore").unwrap();
        assert!(!def.hidden && def.source == AgentSource::Builtin, "等价 override 应被删除而非保留");
    }

    #[test]
    fn set_hidden_enable_keeps_customized_override() {
        let maestro = tempfile::tempdir().unwrap();
        // 用户先自定义覆盖 builtin（不同 prompt），再禁用它。
        let mut custom = AgentDefinition::builtin("explore").unwrap();
        custom.prompt = "自定义调研提示".into();
        custom.hidden = true;
        save(maestro.path(), None, "global", &custom).unwrap();

        let out = set_hidden(maestro.path(), None, &[item_ref("global", "explore")], false);
        assert_eq!(out[0].status, "updated");
        let def = list(maestro.path(), None, "all").into_iter().find(|a| a.name == "explore").unwrap();
        assert!(!def.hidden && def.prompt == "自定义调研提示", "自定义 override 只翻 hidden，不被删除");
    }

    #[test]
    fn delete_many_reports_per_item_and_blocks_builtin() {
        let maestro = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        save(maestro.path(), None, "global", &agent("a", "", AgentSource::Global)).unwrap();
        save(maestro.path(), Some(workspace.path()), "project", &agent("b", "", AgentSource::Project)).unwrap();

        let out = delete_many(maestro.path(), Some(workspace.path()), &[
            item_ref("global", "a"),
            item_ref("project", "b"),
            item_ref("builtin", "explore"),
            item_ref("global", "ghost"),
        ]);
        let status = |name: &str| out.iter().find(|item| item.name == name).unwrap().status.as_str();
        assert_eq!(status("a"), "deleted");
        assert_eq!(status("b"), "deleted");
        assert_eq!(status("explore"), "failed", "builtin 不可删除");
        assert_eq!(status("ghost"), "deleted", "不存在的文件按已删处理（幂等）");
        assert!(list(maestro.path(), None, "global").is_empty());
    }
}
