//! 架构沙盘规则文件的读写：作用域（项目优先、全局回退）、磁盘落点与
//! IPC DTO。求值逻辑在 `domain::architecture`，运行时接入在 ToolRouter。
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::{
    validate_graph, AppResult, ArchitectureGraph, RULES_FILE_NAME,
};

/// 规则作用域：项目 `<workspace>/.maestro/graph_rules.json` 优先，
/// 不存在时回退全局 `~/.maestro/graph_rules.json`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphScope {
    Project,
    Global,
}

impl GraphScope {
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw {
            "project" => Ok(Self::Project),
            "global" => Ok(Self::Global),
            other => Err(format!("未知规则作用域：{other}")),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

/// `get_architecture_graph` 的 IPC 载荷：前端按 `resolved_scope` 高亮
/// 当前标签、按 `effective_scope` 标注「此图正在生效」。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureGraphView {
    /// 本次请求的作用域。
    pub scope: String,
    /// `graph` 实际来自哪个作用域（`scope` 文件不存在时为 null）。
    pub resolved_scope: Option<String>,
    /// 运行期真正生效的作用域（项目文件存在→project，否则全局→global，全无→null）。
    pub effective_scope: Option<String>,
    pub path: Option<String>,
    pub graph: Option<ArchitectureGraph>,
    /// 文件存在但解析/校验失败；保存端不会出现，运行端按「解析失败」处理。
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSaveResult {
    pub path: String,
    pub scope: String,
    /// 空画布保存 → 删除该作用域规则并回退到另一作用域。
    pub removed: bool,
    pub effective_scope: Option<String>,
}

pub fn rules_path(
    scope: GraphScope,
    maestro_dir: &Path,
    workspace: Option<&Path>,
) -> AppResult<PathBuf> {
    match scope {
        GraphScope::Project => workspace
            .map(|root| root.join(".maestro").join(RULES_FILE_NAME))
            .ok_or_else(|| "未打开工作区，无法保存项目级规则".to_string()),
        GraphScope::Global => Ok(maestro_dir.join(RULES_FILE_NAME)),
    }
}

/// 读单个文件：不存在 → `graph=None, error=None`；存在但坏 → `error`。
pub fn load_path(path: &Path) -> (Option<ArchitectureGraph>, Option<String>) {
    if !path.is_file() {
        return (None, None);
    }
    match fs::read_to_string(path) {
        Err(_) => (None, Some("规则文件无法读取".into())),
        Ok(text) => match crate::domain::parse_graph(&text) {
            Ok(graph) => (Some(graph), None),
            Err(error) => (None, Some(error)),
        },
    }
}

/// 生效路径：项目文件存在则项目，否则全局文件存在则全局。
pub fn effective_path(maestro_dir: &Path, workspace: Option<&Path>) -> Option<(GraphScope, PathBuf)> {
    if let Some(root) = workspace {
        let project = root.join(".maestro").join(RULES_FILE_NAME);
        if project.is_file() {
            return Some((GraphScope::Project, project));
        }
    }
    let global = maestro_dir.join(RULES_FILE_NAME);
    global.is_file().then_some((GraphScope::Global, global))
}

/// 组合 `get_architecture_graph` 响应：请求作用域的图 + 当前生效作用域。
pub fn view_for(
    scope: GraphScope,
    maestro_dir: &Path,
    workspace: Option<&Path>,
) -> AppResult<ArchitectureGraphView> {
    let effective_scope = effective_path(maestro_dir, workspace).map(|(scope, _)| scope.as_str().to_string());
    match rules_path(scope, maestro_dir, workspace) {
        Ok(path) => {
            let (graph, error) = load_path(&path);
            Ok(ArchitectureGraphView {
                scope: scope.as_str().to_string(),
                resolved_scope: if graph.is_some() || error.is_some() {
                    Some(scope.as_str().to_string())
                } else {
                    None
                },
                effective_scope,
                path: Some(path.to_string_lossy().into_owned()),
                graph,
                error,
            })
        }
        Err(reason) if scope == GraphScope::Project => Ok(ArchitectureGraphView {
            scope: scope.as_str().to_string(),
            resolved_scope: None,
            effective_scope,
            path: None,
            graph: None,
            error: Some(reason),
        }),
        Err(reason) => Err(reason),
    }
}

/// 校验并落盘；空画布删除该作用域规则文件（触发回退）。
pub fn save(
    scope: GraphScope,
    maestro_dir: &Path,
    workspace: Option<&Path>,
    graph: &ArchitectureGraph,
) -> AppResult<GraphSaveResult> {
    let path = rules_path(scope, maestro_dir, workspace)?;
    let removed = graph.is_empty();
    if removed {
        if path.exists() {
            fs::remove_file(&path).map_err(|_| "无法删除规则文件".to_string())?;
        }
    } else {
        validate_graph(graph)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| "无法创建 .maestro 目录".to_string())?;
        }
        let json = serde_json::to_string_pretty(graph).map_err(|_| "规则序列化失败".to_string())?;
        fs::write(&path, json).map_err(|_| "无法写入规则文件".to_string())?;
    }
    Ok(GraphSaveResult {
        path: path.to_string_lossy().into_owned(),
        scope: scope.as_str().to_string(),
        removed,
        effective_scope: effective_path(maestro_dir, workspace).map(|(scope, _)| scope.as_str().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graph_json() -> ArchitectureGraph {
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [{ "id": "t", "type": "trigger", "position": {"x":0,"y":0}, "data": {"action":"modify","targetDir":"src"} }],
            "edges": [],
        }))
        .unwrap()
    }

    #[test]
    fn project_scope_wins_then_falls_back() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let ws = base.path().join("ws");
        fs::create_dir_all(&global).unwrap();
        fs::create_dir_all(&ws).unwrap();
        assert!(effective_path(&global, Some(&ws)).is_none());
        let graph = graph_json();
        save(GraphScope::Global, &global, Some(&ws), &graph).unwrap();
        assert_eq!(effective_path(&global, Some(&ws)).unwrap().0, GraphScope::Global);
        save(GraphScope::Project, &global, Some(&ws), &graph).unwrap();
        assert_eq!(effective_path(&global, Some(&ws)).unwrap().0, GraphScope::Project);
        save(GraphScope::Project, &global, Some(&ws), &ArchitectureGraph::default()).unwrap();
        assert_eq!(effective_path(&global, Some(&ws)).unwrap().0, GraphScope::Global);
    }

    #[test]
    fn save_roundtrip_and_empty_removes() {
        let base = tempfile::tempdir().unwrap();
        let global = base.path().join("global");
        let ws = base.path().join("ws");
        fs::create_dir_all(&ws).unwrap();
        let graph = graph_json();
        let result = save(GraphScope::Project, &global, Some(&ws), &graph).unwrap();
        assert!(!result.removed);
        assert!(ws.join(".maestro").join(RULES_FILE_NAME).is_file());
        let (loaded, error) = load_path(&ws.join(".maestro").join(RULES_FILE_NAME));
        assert!(error.is_none() && loaded.is_some());
        assert_eq!(effective_path(&global, Some(&ws)).unwrap().0, GraphScope::Project);
        let removed = save(GraphScope::Project, &global, Some(&ws), &ArchitectureGraph::default()).unwrap();
        assert!(removed.removed);
        assert!(!ws.join(".maestro").join(RULES_FILE_NAME).exists());
    }
}
