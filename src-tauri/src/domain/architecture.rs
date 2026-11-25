//! 架构沙盘（Architecture Sandbox）契约图：前端画布序列化的节点-边规则，
//! 以及 Rust 端在任何工具执行前完成求值的引擎。纯数据 + 校验 + 求值 —
//! 文件 IO 在 `services::architecture`，执行接入在 `services::kernel`。
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::AppResult;

pub const GRAPH_VERSION: u32 = 1;
pub const RULES_FILE_NAME: &str = "graph_rules.json";
const MAX_NODES: usize = 128;
const MAX_EDGES: usize = 256;
const MAX_ID_CHARS: usize = 64;
const MAX_FIELD_CHARS: usize = 512;
const MAX_MESSAGE_CHARS: usize = 1024;
const MAX_EXT_CHARS: usize = 128;
const MAX_PARAMS: usize = 20;
const MAX_PARAM_KEY_CHARS: usize = 64;
const MAX_PARAM_VALUE_CHARS: usize = 256;
const MAX_SQUADS: usize = 16;
const MAX_SQUAD_NODES: usize = 32;
const MAX_SQUAD_EDGES: usize = 64;
/// 军团成员上限：并行态一次全派、指挥态也在 enum 内——控制单次 fan-out。
const MAX_SQUAD_MEMBERS: usize = 8;

// ---- 图结构 -----------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureGraph {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    /// 智能体军团（subnet 子沙盘）：`squad` 节点 data.squad 引用这里的键。
    /// 内部只允许 子智能体/扩展 节点——指挥（dispatch 出边）→ 成员树。
    #[serde(default)]
    pub squads: BTreeMap<String, SquadGraph>,
}

impl ArchitectureGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub position: GraphPosition,
    #[serde(default)]
    pub data: GraphNodeData,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct GraphPosition {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

/// `data` 对基础节点是扁平字段；Extension 节点内嵌 `extension` 对象。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GraphNodeData {
    // Trigger
    pub target_dir: Option<String>,
    pub action: Option<TriggerAction>,
    // Requirement
    pub tool_name: Option<String>,
    pub target_file: Option<String>,
    pub update_plan: Option<bool>,
    pub scope: Option<SatisfyScope>,
    /// 事前（调用前必须满足）/ 事后（触发后必须完成验收）。默认 before。
    pub timing: Option<GateTiming>,
    /// 挂起策略：仅 timing=after 有效。batch=欠账期间同类触发放行累积；
    /// strict=改一次必须审一次，同类触发也拦。
    pub mode: Option<GateMode>,
    // Restriction
    pub deny_rule: Option<String>,
    // Action
    pub result: Option<ActionResult>,
    pub message: Option<String>,
    // Extension
    pub extension: Option<ExtensionData>,
    // Subagent 装配节点：绑定的子智能体类型名（如 explore/reviewer）。
    pub agent: Option<String>,
    // Squad 封装节点：引用的军团名（squads 键）。
    pub squad: Option<String>,
    // 军团成员必选性：仅军团内 Subagent 节点使用（默认 required）。
    pub requirement: Option<ExtensionRequirement>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub source_handle: Option<String>,
    #[serde(default)]
    pub target_handle: Option<String>,
}

/// 军团内部子图（Houdini subnet）：仅 子智能体/扩展 节点。
/// dispatch 出边（sourceHandle="dispatch"）的子智能体 = 指挥；其出边
/// 目标 = 可派成员。无 dispatch 边 = 并行军团（spawn 一次全派）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadGraph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Trigger,
    Requirement,
    Restriction,
    Action,
    Extension,
    /// 装配节点（不参与调用求值）：Extension → Subagent 入边声明
    /// 「该类型子智能体获得这些扩展授权」，spawn_subagent 时生效。
    Subagent,
    /// 军团封装节点（subnet）：`data.squad` 引用 `squads` 键；求值透明端点，
    /// spawn_subagent(type=squad名) 按内部编排解析为并行军团或指挥派发。
    Squad,
}

impl NodeKind {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "trigger" => Self::Trigger,
            "requirement" => Self::Requirement,
            "restriction" => Self::Restriction,
            "action" => Self::Action,
            "extension" => Self::Extension,
            "subagent" => Self::Subagent,
            "squad" => Self::Squad,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerAction {
    Modify,
    Read,
    Command,
    Plan,
    Any,
    /// 本轮 Run 的用户消息包含 target_dir 关键词时，对本轮全部工具调用生效。
    Prompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionResult {
    Block,
    Warn,
    /// 放行并把消息拼进工具结果——「执行中时刻提醒」通道（warn 仍属拦截）。
    Remind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Skill,
    Mcp,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionRequirement {
    Required,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SatisfyScope {
    Session,
    Run,
    /// 每次触发重新满足：仅认可「该关卡祖先 Trigger 上次命中之后」的满足调用。
    /// 用于「每次出计划前都要分析」「每次写码前重新查库」这类语义。
    Each,
}

/// 关卡时机：事前=调用前必须满足；事后=触发后必须完成验收。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GateTiming {
    #[default]
    Before,
    After,
}

/// 事后挂起策略：batch=欠账期间同类触发放行累积；strict=同类触发也拦截。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GateMode {
    #[default]
    Batch,
    Strict,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtensionData {
    pub kind: Option<ExtensionKind>,
    pub name: Option<String>,
    pub target: Option<String>,
    /// MCP 具体工具名；缺省表示该 Server 任意工具。
    pub tool: Option<String>,
    pub requirement: Option<ExtensionRequirement>,
    /// `run` 表示每轮 Run 内必须重新满足；默认 session 满足一次即可。
    pub scope: Option<SatisfyScope>,
    /// 事后模式：触发调用后必须实际调用本扩展完成验收。
    pub timing: Option<GateTiming>,
    /// 仅 timing=after 有效：batch 放行同类触发累积，strict 连同类触发也拦。
    pub mode: Option<GateMode>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl ExtensionData {
    pub fn is_required(&self) -> bool {
        self.requirement != Some(ExtensionRequirement::Optional)
    }
    pub fn scope(&self) -> SatisfyScope {
        self.scope.unwrap_or(SatisfyScope::Session)
    }
    pub fn timing(&self) -> GateTiming {
        self.timing.unwrap_or_default()
    }
    pub fn mode(&self) -> GateMode {
        self.mode.unwrap_or_default()
    }
}

impl GraphNodeData {
    fn timing(&self) -> GateTiming {
        self.timing.unwrap_or_default()
    }
    fn mode(&self) -> GateMode {
        self.mode.unwrap_or_default()
    }
}

/// 节点级时机/策略/范围：Requirement 用扁平字段，Extension 用内嵌 ext 字段。
fn node_timing(node: &GraphNode) -> GateTiming {
    match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Extension) => node
            .data
            .extension
            .as_ref()
            .map(|ext| ext.timing())
            .unwrap_or_default(),
        _ => node.data.timing(),
    }
}

fn node_mode(node: &GraphNode) -> GateMode {
    match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Extension) => node
            .data
            .extension
            .as_ref()
            .map(|ext| ext.mode())
            .unwrap_or_default(),
        _ => node.data.mode(),
    }
}

fn node_scope(node: &GraphNode) -> SatisfyScope {
    match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Extension) => node
            .data
            .extension
            .as_ref()
            .map(|ext| ext.scope())
            .unwrap_or(SatisfyScope::Session),
        _ => node.data.scope.unwrap_or(SatisfyScope::Session),
    }
}

// ---- 运行期求值输入 -----------------------------------------------------------

/// 一次待执行工具调用的求值视图。
#[derive(Debug, Clone)]
pub struct PendingCall {
    pub tool: String,
    /// `history_target` 提取的完整目标：文件路径 / 命令文本 / 技能名等。
    pub target: String,
    /// 写入内容或命令文本（Restriction 检查体）；读工具为 None。
    pub content: Option<String>,
}

/// `audit.action='tool_history'` 还原出的成功调用记录。
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub tool: String,
    pub target: String,
    pub run_id: i64,
}

/// 插件清单解析后的真实目标（声明式别名，不执行代码）。
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub kind: ExtensionKind,
    pub target: String,
    pub tool: Option<String>,
}

/// 求值时刻的扩展世界快照；由 ToolRouter 现场装配。
#[derive(Debug, Default)]
pub struct ExtensionSnapshot {
    pub skills: HashSet<String>,
    /// 已配置（无论是否运行）的 MCP Server 名。
    pub mcp_configured: HashSet<String>,
    /// 运行中的 Server -> 已列出工具名。
    pub mcp_tools: BTreeMap<String, HashSet<String>>,
    /// 已安装的插件清单。
    pub plugins: BTreeMap<String, ResolvedPlugin>,
}

// ---- 求值输出 -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Matched,
    Passed,
    Failed,
    Optional,
    Blocked,
    /// 事后验收欠账挂起中（琥珀色高亮）。
    Pending,
    /// Remind 动作已触发：调用放行，消息已附进工具结果。
    Reminded,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceStep {
    pub node_id: String,
    pub status: TraceStatus,
}

#[derive(Debug, Clone, Default)]
pub struct GraphDecision {
    /// 至少一个 Trigger 命中（决定 trace 事件是否发出）。
    pub matched: bool,
    pub blocked: bool,
    /// `warn` 动作同样拦截，只是返回语气不同。
    pub warn: bool,
    pub message: Option<String>,
    /// 放行时附进工具结果的提醒（Remind 动作 + 欠账自动提醒）。
    pub reminder: Option<String>,
    pub trace: Vec<TraceStep>,
}

// ---- 校验 ---------------------------------------------------------------------

pub fn parse_graph(json: &str) -> AppResult<ArchitectureGraph> {
    let graph: ArchitectureGraph = serde_json::from_str(json)
        .map_err(|error| format!("规则 JSON 无法解析：{error}"))?;
    validate_graph(&graph)?;
    Ok(graph)
}

fn bounded(value: &Option<String>, max: usize, what: &str) -> AppResult<()> {
    if let Some(value) = value {
        if value.chars().count() > max {
            return Err(format!("{what} 超过 {max} 字符限制"));
        }
    }
    Ok(())
}

/// 结构校验 + DAG 校验。任何一处非法都让整个文件不可用（保存端拦住，
/// 运行端把「无法解析」视为对副作用工具的整体拦截）。
pub fn validate_graph(graph: &ArchitectureGraph) -> AppResult<()> {
    if graph.version == 0 || graph.version > GRAPH_VERSION {
        return Err(format!("不支持的规则版本 {}", graph.version));
    }
    if graph.nodes.len() > MAX_NODES {
        return Err(format!("节点数超过 {MAX_NODES} 上限"));
    }
    if graph.edges.len() > MAX_EDGES {
        return Err(format!("边数超过 {MAX_EDGES} 上限"));
    }
    // mode/scope 的合法性依赖有效时机（含继承），先算出 effective-after 集合。
    let after_set = effective_after_set(graph);
    let mut ids = HashSet::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let id = node.id.trim();
        if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
            return Err(format!("节点 id 非法：{}", node.id));
        }
        if !ids.insert(node.id.clone()) {
            return Err(format!("节点 id 重复：{id}"));
        }
        let kind = NodeKind::parse(&node.node_type)
            .ok_or_else(|| format!("未知节点类型：{}", node.node_type))?;
        validate_node_data(kind, node, after_set.contains(node.id.as_str()))?;
    }
    let mut edge_ids = HashSet::with_capacity(graph.edges.len());
    for edge in &graph.edges {
        if !edge_ids.insert(edge.id.clone()) {
            return Err(format!("边 id 重复：{}", edge.id));
        }
        let source = graph
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or_else(|| format!("边 {} 的 source 不存在", edge.id))?;
        let target = graph
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or_else(|| format!("边 {} 的 target 不存在", edge.id))?;
        let source_kind = NodeKind::parse(&source.node_type).unwrap();
        let target_kind = NodeKind::parse(&target.node_type).unwrap();
        if source_kind == NodeKind::Action {
            return Err(format!("Action 节点 {} 不能有出边", source.id));
        }
        // 装配终点：子智能体/军团节点只消费扩展入边，自身不向图求值输出。
        if source_kind == NodeKind::Subagent {
            return Err(format!("Subagent 节点 {} 不能有出边", source.id));
        }
        if source_kind == NodeKind::Squad {
            return Err(format!("Squad 节点 {} 不能有出边", source.id));
        }
        if target_kind == NodeKind::Trigger {
            return Err(format!("Trigger 节点 {} 不能作为目标", target.id));
        }
        if matches!(target_kind, NodeKind::Subagent | NodeKind::Squad) {
            if source_kind != NodeKind::Extension {
                return Err(format!("{:?} 节点 {} 只接受扩展节点入边", target_kind, target.id));
            }
            // 装配连线仅表达「授予」，fail 出口在此无意义（未满足在子循环内处理）。
            if edge.source_handle.as_deref().is_some_and(|handle| handle != "pass") {
                return Err(format!("扩展 → 装配终点的连线只能使用 pass 出口（边 {}）", edge.id));
            }
        }
        if let Some(handle) = edge.target_handle.as_deref() {
            if handle != "in" {
                return Err(format!("边 {} 的 targetHandle 只能为 in", edge.id));
            }
        }
        let handle = edge.source_handle.as_deref();
        let optional_extension = source_kind == NodeKind::Extension
            && !source
                .data
                .extension
                .as_ref()
                .map(|ext| ext.is_required())
                .unwrap_or(true);
        match source_kind {
            NodeKind::Trigger => {
                if !matches!(handle, None | Some("out")) {
                    return Err(format!("Trigger 出边只能使用 out Handle（边 {}）", edge.id));
                }
            }
            NodeKind::Requirement | NodeKind::Restriction => {
                if !matches!(handle, None | Some("pass") | Some("fail")) {
                    return Err(format!("条件节点出边只能使用 pass/fail Handle（边 {}）", edge.id));
                }
            }
            NodeKind::Extension => {
                if !matches!(handle, None | Some("pass") | Some("fail")) {
                    return Err(format!("扩展节点出边只能使用 pass/fail Handle（边 {}）", edge.id));
                }
                if optional_extension && matches!(handle, Some("fail")) {
                    return Err(format!("可选扩展节点 {} 不允许 fail 出边", source.id));
                }
            }
            NodeKind::Action | NodeKind::Subagent | NodeKind::Squad => unreachable!(),
        }
    }
    // DAG：Kahn 拓扑，剩环即拒。
    if has_cycle(&graph.nodes, &graph.edges) {
        return Err("规则图存在环（必须是 DAG）".into());
    }
    validate_squads(graph)?;
    Ok(())
}

/// Kahn 拓扑：有环返回 true。主图与军团内图共用。
fn has_cycle(nodes: &[GraphNode], edges: &[GraphEdge]) -> bool {
    let mut indegree: HashMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        *indegree.entry(edge.target.as_str()).or_default() += 1;
        outgoing.entry(edge.source.as_str()).or_default().push(edge.target.as_str());
    }
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop() {
        visited += 1;
        for next in outgoing.get(id).cloned().unwrap_or_default() {
            let degree = indegree.get_mut(next).unwrap();
            *degree -= 1;
            if *degree == 0 {
                queue.push(next);
            }
        }
    }
    visited != nodes.len()
}

/// 军团层校验：squad 节点引用约束 + 每个军团内部结构（指挥/成员/dispatch 边）。
fn validate_squads(graph: &ArchitectureGraph) -> AppResult<()> {
    if graph.squads.len() > MAX_SQUADS {
        return Err(format!("军团数超过 {MAX_SQUADS} 上限"));
    }
    let mut squad_node_names = HashSet::new();
    for node in &graph.nodes {
        if NodeKind::parse(&node.node_type) != Some(NodeKind::Squad) {
            continue;
        }
        let name = node.data.squad.as_deref().map(str::trim).unwrap_or_default();
        if !squad_node_names.insert(name.to_string()) {
            return Err(format!("Squad 节点重名：{name}"));
        }
        if !graph.squads.contains_key(name) {
            return Err(format!("Squad 节点 {} 引用的军团 {name} 不存在于 squads", node.id));
        }
    }
    for (name, squad) in &graph.squads {
        validate_squad(name, squad)?;
    }
    Ok(())
}

/// 军团内部：只允许 子智能体/扩展 节点；dispatch 出边全图至多一个源
/// （指挥），指挥不可被 dispatch 指向；extension 仅 pass→成员；整体 DAG。
fn validate_squad(name: &str, squad: &SquadGraph) -> AppResult<()> {
    if squad.nodes.len() > MAX_SQUAD_NODES {
        return Err(format!("军团 {name} 节点数超过 {MAX_SQUAD_NODES} 上限"));
    }
    if squad.edges.len() > MAX_SQUAD_EDGES {
        return Err(format!("军团 {name} 边数超过 {MAX_SQUAD_EDGES} 上限"));
    }
    let mut ids = HashSet::with_capacity(squad.nodes.len());
    let mut members = 0usize;
    for node in &squad.nodes {
        if node.id.trim().is_empty() || node.id.chars().count() > MAX_ID_CHARS {
            return Err(format!("军团 {name} 节点 id 非法：{}", node.id));
        }
        if !ids.insert(node.id.clone()) {
            return Err(format!("军团 {name} 节点 id 重复：{}", node.id));
        }
        let kind = NodeKind::parse(&node.node_type)
            .ok_or_else(|| format!("军团 {name} 存在未知节点类型：{}", node.node_type))?;
        if !matches!(kind, NodeKind::Subagent | NodeKind::Extension) {
            return Err(format!("军团 {name} 只允许 子智能体/扩展 节点（{} 非法）", node.node_type));
        }
        if kind == NodeKind::Subagent {
            members += 1;
        }
        validate_node_data(kind, node, false)?;
    }
    if members == 0 {
        return Err(format!("军团 {name} 没有成员子智能体"));
    }
    if members > MAX_SQUAD_MEMBERS {
        return Err(format!("军团 {name} 成员数超过 {MAX_SQUAD_MEMBERS} 上限"));
    }
    let mut edge_ids = HashSet::with_capacity(squad.edges.len());
    let mut coordinator: Option<&str> = None;
    let mut dispatch_targets: HashSet<&str> = HashSet::new();
    for edge in &squad.edges {
        if !edge_ids.insert(edge.id.clone()) {
            return Err(format!("军团 {name} 边 id 重复：{}", edge.id));
        }
        let source = squad
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or_else(|| format!("军团 {name} 边 {} 的 source 不存在", edge.id))?;
        let target = squad
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or_else(|| format!("军团 {name} 边 {} 的 target 不存在", edge.id))?;
        let source_kind = NodeKind::parse(&source.node_type).unwrap();
        let target_kind = NodeKind::parse(&target.node_type).unwrap();
        if target_kind != NodeKind::Subagent {
            return Err(format!("军团 {name} 内扩展/成员只能指向子智能体成员（边 {}）", edge.id));
        }
        if edge.target_handle.as_deref().is_some_and(|handle| handle != "in") {
            return Err(format!("军团 {name} 边 {} 的 targetHandle 只能为 in", edge.id));
        }
        match source_kind {
            NodeKind::Extension => {
                if edge.source_handle.as_deref().is_some_and(|handle| handle != "pass") {
                    return Err(format!("军团 {name} 内扩展 → 成员只能使用 pass 出口（边 {}）", edge.id));
                }
            }
            NodeKind::Subagent => {
                if edge.source_handle.as_deref() != Some("dispatch") {
                    return Err(format!("军团 {name} 内成员的调度出边必须使用 dispatch 出口（边 {}）", edge.id));
                }
                if coordinator.replace(source.id.as_str()).is_some_and(|prev| prev != source.id) {
                    return Err(format!("军团 {name} 至多允许一个指挥（dispatch 出边源）"));
                }
                dispatch_targets.insert(target.id.as_str());
            }
            _ => unreachable!(),
        }
    }
    if let Some(coordinator) = coordinator {
        if dispatch_targets.contains(coordinator) {
            return Err(format!("军团 {name} 的指挥不能被 dispatch 指向"));
        }
    }
    if has_cycle(&squad.nodes, &squad.edges) {
        return Err(format!("军团 {name} 内部存在环（必须是 DAG）"));
    }
    Ok(())
}

fn validate_node_data(kind: NodeKind, node: &GraphNode, effective_after: bool) -> AppResult<()> {
    let data = &node.data;
    bounded(&data.target_dir, MAX_FIELD_CHARS, "targetDir")?;
    bounded(&data.tool_name, MAX_EXT_CHARS, "toolName")?;
    bounded(&data.target_file, MAX_FIELD_CHARS, "targetFile")?;
    bounded(&data.deny_rule, MAX_FIELD_CHARS, "denyRule")?;
    bounded(&data.message, MAX_MESSAGE_CHARS, "message")?;
    // scope=each 仅前置关卡可用；事后义务的欠账模型天然按每次触发欠账，
    // each 在其上无意义。
    let each_scope = data.scope == Some(SatisfyScope::Each)
        || data
            .extension
            .as_ref()
            .is_some_and(|ext| ext.scope == Some(SatisfyScope::Each));
    if each_scope {
        if !matches!(kind, NodeKind::Requirement | NodeKind::Extension) {
            return Err(format!("{} 的 scope=each 仅对前置要求/扩展有效", node.id));
        }
        if effective_after {
            return Err(format!("{} 是事后关卡，scope=each 无意义（欠账天然按触发欠账）", node.id));
        }
    }
    match kind {
        NodeKind::Trigger => {
            if data.action.is_none() {
                return Err(format!("Trigger {} 缺少 action", node.id));
            }
        }
        NodeKind::Requirement => {
            if data.tool_name.is_none() && data.target_file.is_none() && data.update_plan != Some(true) {
                return Err(format!("Requirement {} 需要 toolName、targetFile 或 updatePlan", node.id));
            }
            if data.mode.is_some() && !effective_after {
                return Err(format!("Requirement {} 的 mode 仅在事后关卡上有效", node.id));
            }
        }
        NodeKind::Restriction => {
            if data.deny_rule.as_deref().map(str::trim).unwrap_or("").is_empty() {
                return Err(format!("Restriction {} 需要非空 denyRule", node.id));
            }
        }
        NodeKind::Action => {
            if data.result.is_none() {
                return Err(format!("Action {} 缺少 result（block/warn/remind）", node.id));
            }
        }
        NodeKind::Subagent => {
            // 与 adapters::agents::save 同款命名字符集，保证绑定名可命中磁盘定义。
            let name = data
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("Subagent {} 缺少 agent 类型名", node.id))?;
            if name.chars().count() > 64
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(format!("Subagent {} 的 agent 名必须为字母/数字/-/_", node.id));
            }
        }
        NodeKind::Squad => {
            let name = data
                .squad
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("Squad {} 缺少 squad 军团名", node.id))?;
            if name.chars().count() > 64
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(format!("Squad {} 的军团名必须为字母/数字/-/_", node.id));
            }
        }
        NodeKind::Extension => {
            let ext = data
                .extension
                .as_ref()
                .ok_or_else(|| format!("Extension {} 缺少 extension 配置", node.id))?;
            for (value, what) in [
                (&ext.name, "name"),
                (&ext.target, "target"),
            ] {
                let text = value
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| format!("Extension {} 缺少 {what}", node.id))?;
                if text.chars().count() > MAX_EXT_CHARS {
                    return Err(format!("Extension {} 的 {what} 超长", node.id));
                }
            }
            if ext.kind.is_none() {
                return Err(format!("Extension {} 缺少 kind（skill/mcp/plugin）", node.id));
            }
            if ext.kind == Some(ExtensionKind::Skill) && ext.tool.is_some() {
                return Err(format!("Extension {}（skill）不允许 tool 字段", node.id));
            }
            if ext.params.len() > MAX_PARAMS {
                return Err(format!("Extension {} 的 params 超过 {MAX_PARAMS} 项", node.id));
            }
            if ext.mode.is_some() && !effective_after {
                return Err(format!("Extension {} 的 mode 仅在事后关卡上有效", node.id));
            }
            for (key, value) in &ext.params {
                if key.is_empty() || key.chars().count() > MAX_PARAM_KEY_CHARS {
                    return Err(format!("Extension {} 的 params 键非法", node.id));
                }
                if value.chars().count() > MAX_PARAM_VALUE_CHARS {
                    return Err(format!("Extension {} 的 params 值超长", node.id));
                }
            }
        }
    }
    Ok(())
}

// ---- 求值 ---------------------------------------------------------------------

const WRITE_TOOLS: &[&str] = &["write_file", "apply_patch"];
const READ_TOOLS: &[&str] = &["read_file", "list_dir", "glob_files", "grep_files", "read_session", "task_output"];
const PATH_READ_TOOLS: &[&str] = &["read_file", "list_dir"];
const COMMAND_TOOLS: &[&str] = &["run_command", "task_stop"];
const PLAN_TOOLS: &[&str] = &["update_plan", "ask_user"];
/// 验收欠账期间的无害放行集：查看/规划/提问/加载技能都不构成实质动作，
/// 不满足验收也不拦截（满足验收的调用另有 satisfies 判定放行）。
const POST_EXEMPT: &[&str] = &[
    "read_file", "list_dir", "glob_files", "grep_files", "read_session", "task_output",
    "update_plan", "ask_user", "load_skill",
];

/// 规范化路径：`\`→`/`、去 `./` 前缀与结尾 `/`。
pub fn normalize_rel(value: &str) -> String {
    let mut text = value.trim().replace('\\', "/");
    while let Some(rest) = text.strip_prefix("./") {
        text = rest.to_string();
    }
    while text.starts_with('/') {
        text.remove(0);
    }
    while text.ends_with('/') {
        text.pop();
    }
    text
}

/// `dir` 为空 → 命中全部；否则要求 `target == dir` 或 `target` 在 `dir/` 之下。
pub fn path_matches(dir: &str, target: &str) -> bool {
    let dir = normalize_rel(dir);
    if dir.is_empty() {
        return true;
    }
    let target = normalize_rel(target);
    target == dir || target.starts_with(&format!("{dir}/"))
}

/// Restriction 命中：`deny_rule` 在内容中出现（`/` 与 `\` 两种写法都算）。
pub fn deny_hit(needle: &str, hay: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    hay.contains(needle)
        || hay.replace('\\', "/").contains(&needle.replace('\\', "/"))
        || hay.contains(&needle.replace('/', "\\"))
}

fn scope_hits(scope: SatisfyScope, entry: &HistoryEntry, run_id: i64) -> bool {
    match scope {
        SatisfyScope::Session => true,
        // Each 在欠账扫描中无意义（validate 已拒），按 Run 防御处理。
        SatisfyScope::Run | SatisfyScope::Each => entry.run_id == run_id,
    }
}

fn trigger_matches(data: &GraphNodeData, call: &PendingCall, user_message: &str) -> bool {
    let dir = data.target_dir.as_deref().map(normalize_rel).unwrap_or_default();
    match data.action.unwrap_or(TriggerAction::Modify) {
        TriggerAction::Modify => WRITE_TOOLS.contains(&call.tool.as_str()) && path_matches(&dir, &call.target),
        TriggerAction::Read => {
            if !READ_TOOLS.contains(&call.tool.as_str()) {
                return false;
            }
            // 只有带路径参数的读工具可以做目录约束；glob/grep 等只匹配空目录条件。
            dir.is_empty() || (PATH_READ_TOOLS.contains(&call.tool.as_str()) && path_matches(&dir, &call.target))
        }
        TriggerAction::Command => {
            COMMAND_TOOLS.contains(&call.tool.as_str()) && (dir.is_empty() || call.target.contains(&dir))
        }
        TriggerAction::Plan => PLAN_TOOLS.contains(&call.tool.as_str()),
        TriggerAction::Any => {
            dir.is_empty() || (!call.target.is_empty() && path_matches(&dir, &call.target))
        }
        TriggerAction::Prompt => {
            dir.is_empty() || user_message.to_lowercase().contains(&dir.to_lowercase())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ExtState {
    Used,
    Unused,
    Missing(String),
}

/// 一条事后验收义务：祖先 Trigger 触发后必须满足的关卡（Requirement/Extension）。
struct Obligation<'a> {
    node: &'a GraphNode,
    /// 反向 BFS 得到的祖先 Trigger data（arming 判定）。
    triggers: Vec<GraphNodeData>,
    /// fail 分支直连的 Action（拦截文案来源）。
    action: Option<&'a GraphNode>,
}

/// Run 末欠账查询返回项。
#[derive(Debug, Clone)]
pub struct ArmedObligation {
    pub node_id: String,
    pub message: String,
}

/// (passed, blocked, 阻塞原因, 轨迹) —— 各节点义务求值的记忆化结果。
type EvalMemo<'a> = HashMap<&'a str, (bool, bool, Option<String>, Vec<TraceStep>)>;

struct Eval<'a> {
    history: &'a [HistoryEntry],
    run_id: i64,
    ext: &'a ExtensionSnapshot,
    by_id: HashMap<&'a str, &'a GraphNode>,
    out_edges: HashMap<&'a str, Vec<&'a GraphEdge>>,
    memo: EvalMemo<'a>,
    /// 本次待执行调用：Restriction 检查体 + 事后满足判定共用。
    call: &'a PendingCall,
    /// 已挂起的事后义务节点 id（evaluate 前置计算）。
    armed: HashSet<&'a str>,
    /// 有效事后关卡集合（显式 after + 沿 pass 边继承，evaluate 前置计算）。
    after: &'a HashSet<String>,
    /// 各关卡祖先 Trigger data（each scope 的「上次触发」下界）。
    triggers: HashMap<&'a str, Vec<GraphNodeData>>,
    /// 本轮用户消息（prompt 触发器在祖先判定中需要）。
    user_message: &'a str,
}

/// 有效事后关卡集合：事后语义沿 pass 出边向下游 Requirement/Extension 继承——
/// 挂在「事后关卡·通过」下、自身未显式设置 timing 的关卡同样按事后求值
/// （用户把验收 Skill 挂在事后关卡下的直觉语义）。显式 timing=before 可钉住为事前；
/// Restriction 不传播（不是时机载体）。
pub fn effective_after_set(graph: &ArchitectureGraph) -> HashSet<String> {
    let is_gate = |node: &GraphNode| matches!(NodeKind::parse(&node.node_type), Some(NodeKind::Requirement | NodeKind::Extension));
    let raw_timing = |node: &GraphNode| -> Option<GateTiming> {
        if NodeKind::parse(&node.node_type) == Some(NodeKind::Extension) {
            node.data.extension.as_ref().and_then(|ext| ext.timing)
        } else {
            node.data.timing
        }
    };
    let mut set: HashSet<String> = graph
        .nodes
        .iter()
        .filter(|node| is_gate(node) && node_timing(node) == GateTiming::After)
        .map(|node| node.id.clone())
        .collect();
    let mut queue: Vec<String> = set.iter().cloned().collect();
    while let Some(id) = queue.pop() {
        for edge in graph.edges.iter().filter(|edge| edge.source == id) {
            // 与 Eval::children 同一规则：缺省 handle 按 pass 计。
            if edge.source_handle.as_deref().unwrap_or("pass") != "pass" {
                continue;
            }
            let Some(child) = graph.nodes.iter().find(|node| node.id == edge.target) else {
                continue;
            };
            // 显式时机（before/after）不再继承；after 本身已在集合中。
            if !is_gate(child) || raw_timing(child).is_some() {
                continue;
            }
            if set.insert(child.id.clone()) {
                queue.push(child.id.clone());
            }
        }
    }
    set
}

impl<'a> Eval<'a> {
    fn children(&self, id: &str, handle: &str) -> Vec<&'a str> {
        self.out_edges
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|edge| edge.source_handle.as_deref().unwrap_or(handle) == handle)
                    .map(|edge| edge.target.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// scope → 历史窗口：(跳过前 N 条, 只看本轮)。
    /// Each：跳过到该关卡祖先 Trigger 上一次命中之后——满足调用必须比上次触发新。
    fn window(&self, node: &GraphNode) -> (usize, bool) {
        match node_scope(node) {
            SatisfyScope::Session => (0, false),
            SatisfyScope::Run => (0, true),
            SatisfyScope::Each => (self.last_trigger_hit(node).map(|i| i + 1).unwrap_or(0), false),
        }
    }

    /// 该节点祖先 Trigger 上次在历史中命中的下标（each scope 用）。
    fn last_trigger_hit(&self, node: &GraphNode) -> Option<usize> {
        let ancestors = self.triggers.get(node.id.as_str())?;
        self.history.iter().rposition(|entry| {
            let call = PendingCall { tool: entry.tool.clone(), target: entry.target.clone(), content: None };
            ancestors.iter().any(|data| trigger_matches(data, &call, self.user_message))
        })
    }

    fn history_has(&self, window: (usize, bool), tool_match: impl Fn(&str) -> bool, target_match: impl Fn(&str) -> bool) -> bool {
        self.history
            .iter()
            .enumerate()
            .any(|(index, entry)| index >= window.0 && (!window.1 || entry.run_id == self.run_id) && tool_match(&entry.tool) && target_match(&entry.target))
    }

    fn requirement_met(&self, node: &GraphNode) -> bool {
        let data = &node.data;
        let window = self.window(node);
        if data.update_plan == Some(true) {
            return self.history_has(window, |tool| tool == "update_plan", |_| true);
        }
        match (data.tool_name.as_deref(), data.target_file.as_deref()) {
            (Some(tool), Some(file)) => {
                if COMMAND_TOOLS.contains(&tool) {
                    // 命令类工具的目标是命令文本而非路径——按子串判定
                    // （与 Command 触发器同规则），否则「跑 cargo test」判不出来。
                    let needle = normalize_rel(file);
                    self.history_has(window, |t| t == tool, |target| target.contains(&needle))
                } else {
                    self.history_has(window, |t| t == tool, |target| path_matches(file, target))
                }
            }
            (Some(tool), None) => self.history_has(window, |t| t == tool, |_| true),
            (None, Some(file)) => self.history_has(window, |t| t == "read_file", |target| path_matches(file, target)),
            (None, None) => false,
        }
    }

    fn extension_state(&self, node: &GraphNode, ext: &ExtensionData) -> ExtState {
        let kind = ext.kind.unwrap_or(ExtensionKind::Skill);
        let target = ext.target.as_deref().unwrap_or_default();
        let window = self.window(node);
        match kind {
            ExtensionKind::Skill => self.skill_state(target, window),
            ExtensionKind::Mcp => self.mcp_state(target, ext.tool.as_deref(), window),
            ExtensionKind::Plugin => match self.ext.plugins.get(target) {
                None => ExtState::Missing(format!("插件 {target} 未安装或已失效")),
                Some(resolved) => match resolved.kind {
                    ExtensionKind::Skill => self.skill_state(&resolved.target, window),
                    ExtensionKind::Mcp => self.mcp_state(&resolved.target, resolved.tool.as_deref().or(ext.tool.as_deref()), window),
                    ExtensionKind::Plugin => ExtState::Missing(format!("插件 {target} 目标非法")),
                },
            },
        }
    }

    fn skill_state(&self, name: &str, window: (usize, bool)) -> ExtState {
        if !self.ext.skills.contains(name) {
            return ExtState::Missing(format!("技能 {name} 未安装"));
        }
        if self.history_has(window, |tool| tool == "load_skill", |target| target == name) {
            ExtState::Used
        } else {
            ExtState::Unused
        }
    }

    fn mcp_state(&self, server: &str, tool: Option<&str>, window: (usize, bool)) -> ExtState {
        if !self.ext.mcp_configured.contains(server) {
            return ExtState::Missing(format!("MCP 服务 {server} 未配置"));
        }
        if !self.ext.mcp_tools.contains_key(server) {
            return ExtState::Missing(format!("MCP 服务 {server} 未运行"));
        }
        let prefix = format!("mcp__{server}__");
        let used = match tool {
            Some(tool) => self.history_has(window, |name| name == format!("{prefix}{tool}"), |_| true),
            None => self.history_has(window, |name| name.starts_with(&prefix), |_| true),
        };
        if used {
            ExtState::Used
        } else {
            ExtState::Unused
        }
    }

    /// 该 pending call 是否满足「事后」关卡——与 requirement_met/extension_state
    /// 同一套谓词，方向相反：查调用本身而不是查历史。
    fn call_satisfies(&self, node: &GraphNode, call: &PendingCall) -> bool {
        let data = &node.data;
        match NodeKind::parse(&node.node_type) {
            Some(NodeKind::Requirement) => {
                if data.update_plan == Some(true) {
                    return call.tool == "update_plan";
                }
                match (data.tool_name.as_deref(), data.target_file.as_deref()) {
                    (Some(tool), Some(file)) => {
                        call.tool == tool
                            && if COMMAND_TOOLS.contains(&tool) {
                                call.target.contains(&normalize_rel(file))
                            } else {
                                path_matches(file, &call.target)
                            }
                    }
                    (Some(tool), None) => call.tool == tool,
                    (None, Some(file)) => call.tool == "read_file" && path_matches(file, &call.target),
                    (None, None) => false,
                }
            }
            Some(NodeKind::Extension) => data
                .extension
                .as_ref()
                .is_some_and(|ext| self.call_satisfies_extension(ext, call)),
            _ => false,
        }
    }

    fn call_satisfies_extension(&self, ext: &ExtensionData, call: &PendingCall) -> bool {
        let kind = ext.kind.unwrap_or(ExtensionKind::Skill);
        let target = ext.target.as_deref().unwrap_or_default();
        let (kind, target, tool) = match kind {
            ExtensionKind::Plugin => match self.ext.plugins.get(target) {
                None => return false,
                Some(resolved) => (
                    resolved.kind,
                    resolved.target.as_str(),
                    resolved.tool.as_deref().or(ext.tool.as_deref()),
                ),
            },
            kind => (kind, target, ext.tool.as_deref()),
        };
        match kind {
            ExtensionKind::Skill => call.tool == "load_skill" && call.target == target,
            ExtensionKind::Mcp => match tool {
                Some(tool) => call.tool == format!("mcp__{target}__{tool}"),
                None => call.tool.starts_with(&format!("mcp__{target}__")),
            },
            ExtensionKind::Plugin => false,
        }
    }

    /// 义务挂起判定：顺序扫描 scope 内历史——满足调用还账（disarm），
    /// 触发调用欠账（arm）。满足调用永不自欠账，any Trigger 也不会死循环。
    fn obligation_armed(&self, obligation: &Obligation, user_message: &str) -> bool {
        let scope = node_scope(obligation.node);
        let mut armed = false;
        for entry in self.history {
            if !scope_hits(scope, entry, self.run_id) {
                continue;
            }
            let call = PendingCall {
                tool: entry.tool.clone(),
                target: entry.target.clone(),
                content: None,
            };
            if self.call_satisfies(obligation.node, &call) {
                armed = false;
                continue;
            }
            if obligation
                .triggers
                .iter()
                .any(|data| trigger_matches(data, &call, user_message))
            {
                armed = true;
            }
        }
        armed
    }

    fn is_optional(&self, node: &GraphNode) -> bool {
        NodeKind::parse(&node.node_type) == Some(NodeKind::Extension)
            && node
                .data
                .extension
                .as_ref()
                .is_some_and(|ext| !ext.is_required())
    }

    /// 扩展缺失原因：事后义务还不掉时给出具体原因而非通用拦截语。
    fn postcondition_missing(&self, node: &GraphNode) -> Option<String> {
        let ext = node.data.extension.as_ref()?;
        match self.extension_state(node, ext) {
            ExtState::Missing(reason) => Some(reason),
            _ => None,
        }
    }

    fn obligation_message(&self, obligation: &Obligation) -> String {
        if let Some(reason) = self.postcondition_missing(obligation.node) {
            return reason;
        }
        if let Some(action) = obligation.action {
            if let Some(message) = action
                .data
                .message
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                // 占位符按本义务的关卡节点渲染——共享 Action 不串名。
                return render_message(message, obligation.node);
            }
        }
        default_after_message(obligation.node)
    }

    /// 多欠账清单条目：缺失原因 ?? 自定义渲染文案 ?? 短描述（不重复整句前缀）。
    fn obligation_short(&self, obligation: &Obligation) -> String {
        if let Some(reason) = self.postcondition_missing(obligation.node) {
            return reason;
        }
        obligation
            .action
            .and_then(|action| action.data.message.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| render_message(text, obligation.node))
            .unwrap_or_else(|| describe_obligation(obligation.node))
    }

    /// 事后节点对「命中其祖先 Trigger 的调用」的判定。
    /// 满足调用放行；缺失按必需性拦/放；欠账时 strict 拦、batch 标记待验收放行。
    fn check_postcondition(&self, node: &GraphNode) -> (bool, TraceStatus, String) {
        let optional = self.is_optional(node);
        if self.call_satisfies(node, self.call) {
            return (true, TraceStatus::Passed, String::new());
        }
        if let Some(reason) = self.postcondition_missing(node) {
            return if optional {
                (true, TraceStatus::Optional, String::new())
            } else {
                (false, TraceStatus::Failed, reason)
            };
        }
        if !self.armed.contains(node.id.as_str()) {
            // 首个触发调用：欠账从它开始，本笔放行。
            return (true, TraceStatus::Passed, String::new());
        }
        if optional {
            return (true, TraceStatus::Optional, String::new());
        }
        if node_mode(node) == GateMode::Strict {
            return (false, TraceStatus::Failed, default_after_message(node));
        }
        (true, TraceStatus::Pending, String::new())
    }

    /// (passed, status, default_deny_message)
    fn check(&self, node: &GraphNode) -> (bool, TraceStatus, String) {
        let data = &node.data;
        match NodeKind::parse(&node.node_type).unwrap() {
            NodeKind::Requirement | NodeKind::Extension if self.after.contains(node.id.as_str()) => {
                self.check_postcondition(node)
            }
            NodeKind::Requirement => {
                let met = self.requirement_met(node);
                (
                    met,
                    if met { TraceStatus::Passed } else { TraceStatus::Failed },
                    format!("必须先满足前置要求：{}", describe_requirement(data)),
                )
            }
            NodeKind::Restriction => {
                let rule = data.deny_rule.as_deref().unwrap_or_default();
                let violated = self
                    .call
                    .content
                    .as_deref()
                    .is_some_and(|content| deny_hit(rule, content));
                (
                    !violated,
                    if violated { TraceStatus::Failed } else { TraceStatus::Passed },
                    format!("违反架构限制：禁止 {rule}"),
                )
            }
            NodeKind::Extension => {
                let ext = data.extension.as_ref().unwrap();
                match self.extension_state(node, ext) {
                    ExtState::Used => (true, TraceStatus::Passed, String::new()),
                    ExtState::Unused => {
                        if ext.is_required() {
                            (false, TraceStatus::Failed, default_extension_message(ext))
                        } else {
                            (true, TraceStatus::Optional, String::new())
                        }
                    }
                    ExtState::Missing(reason) => {
                        if ext.is_required() {
                            (false, TraceStatus::Failed, reason)
                        } else {
                            (true, TraceStatus::Optional, String::new())
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    fn walk(&mut self, id: &'a str) -> (bool, bool, Option<String>, Vec<TraceStep>) {
        if let Some(hit) = self.memo.get(id) {
            return hit.clone();
        }
        let node = *self.by_id.get(id).expect("validated node id");
        let kind = NodeKind::parse(&node.node_type).unwrap();
        let mut trace = Vec::new();
        let result = match kind {
            NodeKind::Trigger => {
                trace.push(TraceStep { node_id: id.into(), status: TraceStatus::Matched });
                self.walk_children(id, "out", trace)
            }
            NodeKind::Action => {
                let result = node.data.result.unwrap_or(ActionResult::Block);
                // 空文案返回 None——默认值由 walk_children 按上游关卡派生（自动获取）。
                let message = node
                    .data
                    .message
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string);
                if result == ActionResult::Remind {
                    // 放行+附言：非阻断消息仅出自此分支（evaluate 据此归入 reminder）。
                    trace.push(TraceStep { node_id: id.into(), status: TraceStatus::Reminded });
                    return (false, false, message, trace);
                }
                trace.push(TraceStep { node_id: id.into(), status: TraceStatus::Blocked });
                let warn = result == ActionResult::Warn;
                (true, warn, message, trace)
            }
            NodeKind::Subagent | NodeKind::Squad => {
                // 装配终点：求值透明——连线只声明授权/编排，不拦截、不产生 trace 步骤。
                (false, false, None, trace)
            }
            NodeKind::Requirement | NodeKind::Restriction | NodeKind::Extension => {
                let (passed, status, default_message) = self.check(node);
                trace.push(TraceStep { node_id: id.into(), status });
                if passed {
                    let children = self.children(id, "pass");
                    if children.is_empty() {
                        (false, false, None, trace)
                    } else {
                        self.walk_children(id, "pass", trace)
                    }
                } else {
                    let children = self.children(id, "fail");
                    if children.is_empty() {
                        (true, false, Some(default_message), trace)
                    } else {
                        self.walk_children(id, "fail", trace)
                    }
                }
            }
        };
        self.memo.insert(id, result.clone());
        result
    }

    /// 访问某个 Handle 下的全部子节点（AND 语义：任一分支拦截即整体拦截，
    /// trace 合并所有已评估路径）。非阻断消息（Remind 动作）一并向上传递。
    fn walk_children(&mut self, id: &str, handle: &str, mut trace: Vec<TraceStep>) -> (bool, bool, Option<String>, Vec<TraceStep>) {
        // merge 聚合：同一 handle 下全部阻断分支的消息收集为清单（不再只报第一条）。
        let mut blockers: Vec<(bool, Option<String>)> = Vec::new();
        let mut reminders: Vec<String> = Vec::new();
        // Action 文案占位符按上游节点渲染：fail 分支=失败关卡、pass/out=通过关卡/触发器。
        // 先取出父节点避免闭包借用与 walk 的可变借用冲突。
        let parent = self.by_id.get(id).copied();
        let render = |m: Option<String>| {
            m.map(|text| parent.map(|node| render_message(&text, node)).unwrap_or(text))
        };
        for child in self.children(id, handle) {
            let child_is_action = self
                .by_id
                .get(child)
                .and_then(|n| NodeKind::parse(&n.node_type))
                == Some(NodeKind::Action);
            let (b, w, m, mut steps) = self.walk(child);
            trace.append(&mut steps);
            // Action 空文案按上游节点自动派生；非空文案过占位符渲染。
            let rendered = match m {
                Some(_) => render(m),
                None if child_is_action && b => parent.map(|n| default_gate_message(n, w)),
                None if child_is_action => parent.map(default_remind_message),
                None => None,
            };
            if b {
                blockers.push((w, rendered));
            } else if let Some(text) = rendered {
                reminders.push(text);
            }
        }
        if blockers.is_empty() {
            return (false, false, (!reminders.is_empty()).then(|| reminders.join("\n")), trace);
        }
        // warn 聚合：任一 block 分支即整体 block；全部 warn 才是 warn。
        let warn = blockers.iter().all(|(w, _)| *w);
        let messages: Vec<String> = blockers.into_iter().filter_map(|(_, m)| m).collect();
        (true, warn, Some(merge_messages(&messages)), trace)
    }
}

fn describe_requirement(data: &GraphNodeData) -> String {
    if data.update_plan == Some(true) {
        return "先调用 update_plan 提交计划".into();
    }
    match (data.tool_name.as_deref(), data.target_file.as_deref()) {
        (Some(tool), Some(file)) => format!("先调用 {tool}（{file}）"),
        (Some(tool), None) => format!("先调用 {tool}"),
        (None, Some(file)) => format!("先读取 {file}"),
        (None, None) => "前置条件".into(),
    }
}

fn default_extension_message(ext: &ExtensionData) -> String {
    let target = ext.target.as_deref().unwrap_or_default();
    let params_hint = if ext.params.is_empty() {
        String::new()
    } else {
        let pairs = ext
            .params
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("，");
        format!("（参数提示：{pairs}）")
    };
    match ext.kind.unwrap_or(ExtensionKind::Skill) {
        ExtensionKind::Skill => format!("你必须先调用 load_skill {target} 才能继续。{params_hint}"),
        ExtensionKind::Mcp => match ext.tool.as_deref() {
            Some(tool) => format!("你必须先调用 mcp__{target}__{tool} 才能继续。{params_hint}"),
            None => format!("你必须先调用 MCP 服务 {target} 的任意工具才能继续。{params_hint}"),
        },
        ExtensionKind::Plugin => format!("你必须先完成插件 {target} 要求的调用才能继续。{params_hint}"),
    }
}

/// 事后义务的验收调用描述（Run 末追问与缺省拦截文案共用）。
/// 关卡/扩展义务的统一描述（拦截文案、欠账清单、契约装配段共用同一口径）。
pub fn describe_obligation(node: &GraphNode) -> String {
    let data = &node.data;
    match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Requirement) => {
            if data.update_plan == Some(true) {
                return "调用 update_plan 提交计划".into();
            }
            match (data.tool_name.as_deref(), data.target_file.as_deref()) {
                (Some(tool), Some(file)) => format!("调用 {tool}（{file}）"),
                (Some(tool), None) => format!("调用 {tool}"),
                (None, Some(file)) => format!("读取 {file}"),
                (None, None) => "完成指定调用".into(),
            }
        }
        Some(NodeKind::Extension) => {
            let ext = data.extension.as_ref();
            let target = ext.and_then(|e| e.target.as_deref()).unwrap_or_default();
            match ext.and_then(|e| e.kind).unwrap_or(ExtensionKind::Skill) {
                ExtensionKind::Skill => format!("调用 load_skill {target}"),
                ExtensionKind::Mcp => match ext.and_then(|e| e.tool.as_deref()) {
                    Some(tool) => format!("调用 mcp__{target}__{tool}"),
                    None => format!("调用 MCP 服务 {target} 的任意工具"),
                },
                ExtensionKind::Plugin => format!("完成插件 {target} 要求的调用"),
            }
        }
        _ => "完成验收调用".into(),
    }
}

fn default_after_message(node: &GraphNode) -> String {
    format!("验收未完成前不能进行其他操作：请先{}", describe_obligation(node))
}

// ---- 子智能体装配（Extension → Subagent 绑定查询，不参与求值） -------------------

/// 一条装配绑定：扩展节点（插件已按清单展开）授权给子智能体的能力。
#[derive(Debug, Clone)]
pub struct BoundExtension {
    pub kind: ExtensionKind,
    /// 解析后的目标：Skill 名或 MCP Server 名。
    pub target: String,
    /// MCP 具体工具名；缺省表示该 Server 任意工具。
    pub tool: Option<String>,
    pub required: bool,
    pub params: BTreeMap<String, String>,
    /// 插件无法解析时的告警（清单未安装/损坏）——装配缺失不静默授权。
    pub missing: Option<String>,
}

/// 扩展声明解析为真实绑定（插件按清单展开，缺失带 missing 告警）。
fn resolve_ext(
    ext: &ExtensionData,
    plugins: &BTreeMap<String, ResolvedPlugin>,
) -> (ExtensionKind, String, Option<String>, Option<String>) {
    let target = ext.target.as_deref().unwrap_or_default();
    match ext.kind.unwrap_or(ExtensionKind::Skill) {
        ExtensionKind::Plugin => match plugins.get(target) {
            Some(plugin) => (plugin.kind, plugin.target.clone(), plugin.tool.clone().or(ext.tool.clone()), None),
            None => (ExtensionKind::Plugin, target.to_string(), None, Some(format!("插件 {target} 未安装或清单损坏"))),
        },
        kind => (kind, target.to_string(), ext.tool.clone(), None),
    }
}

/// Extension → target_ids 入边集合（merge 输入语义）：插件按 `plugins` 清单
/// 展开为底层 Skill/MCP；同一目标去重，required 优先于 optional。
/// 主图与军团内图共用（装配语义一致）。
fn collect_bound(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
    target_ids: &HashSet<&str>,
    plugins: &BTreeMap<String, ResolvedPlugin>,
) -> Vec<BoundExtension> {
    if target_ids.is_empty() {
        return Vec::new();
    }
    let by_id: HashMap<&str, &GraphNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut out: Vec<BoundExtension> = Vec::new();
    for edge in edges {
        if !target_ids.contains(edge.target.as_str()) {
            continue;
        }
        let Some(source) = by_id.get(edge.source.as_str()) else { continue };
        if NodeKind::parse(&source.node_type) != Some(NodeKind::Extension) {
            continue;
        }
        let Some(ext) = source.data.extension.as_ref() else { continue };
        let required = ext.is_required();
        let (kind, target, tool, missing) = resolve_ext(ext, plugins);
        // 去重键不含 required：同名绑定合并为更严格的 required。
        let key = (kind, target.clone(), tool.clone());
        if let Some(existing) = out.iter_mut().find(|item| {
            item.kind == key.0 && item.target == key.1 && item.tool == key.2
        }) {
            existing.required |= required;
            continue;
        }
        out.push(BoundExtension { kind: key.0, target: key.1, tool: key.2, required, params: ext.params.clone(), missing });
    }
    out
}

/// 图中绑给 `agent` 类型子智能体的全部扩展：Extension → Subagent(name=agent)
/// 入边集合（merge 输入语义）。插件按 `plugins` 清单展开为底层 Skill/MCP；
/// 同一目标去重，required 优先于 optional。
pub fn subagent_bindings(
    graph: &ArchitectureGraph,
    agent: &str,
    plugins: &BTreeMap<String, ResolvedPlugin>,
) -> Vec<BoundExtension> {
    let ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|node| {
            NodeKind::parse(&node.node_type) == Some(NodeKind::Subagent)
                && node.data.agent.as_deref().map(str::trim) == Some(agent)
        })
        .map(|node| node.id.as_str())
        .collect();
    collect_bound(&graph.nodes, &graph.edges, &ids, plugins)
}

/// 主图 Extension → Squad(name) 入边：军团级授权（对指挥与全部成员生效）。
pub fn squad_bindings(
    graph: &ArchitectureGraph,
    squad: &str,
    plugins: &BTreeMap<String, ResolvedPlugin>,
) -> Vec<BoundExtension> {
    let ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|node| {
            NodeKind::parse(&node.node_type) == Some(NodeKind::Squad)
                && node.data.squad.as_deref().map(str::trim) == Some(squad)
        })
        .map(|node| node.id.as_str())
        .collect();
    collect_bound(&graph.nodes, &graph.edges, &ids, plugins)
}

// ---- 智能体军团（squad 编排解析，spawn_subagent 时用） -----------------------------

/// 军团成员：引用一个 AgentDefinition 的子智能体节点 + 军团内装配授权。
#[derive(Debug, Clone)]
pub struct SquadMember {
    /// 成员的 `data.agent`（spawn_subagent 的 subagent_type 候选）。
    pub agent: String,
    /// `data.requirement`（默认 required）：指挥态未派被追问；并行态忽略（全员皆派）。
    pub required: bool,
    /// 军团内 Extension → 该成员的装配授权。
    pub grants: Vec<BoundExtension>,
}

/// 军团编排解析：有指挥（dispatch 出边源）→ 嵌套派发；
/// 无指挥 → 并行军团（成员全部并行派生）。
#[derive(Debug, Clone)]
pub struct SquadPlan {
    /// 指挥成员的 agent 类型（dispatch 出边源；校验保证至多一个）。
    pub coordinator: Option<String>,
    /// 军团内 Extension → 指挥的装配授权（指挥也是子智能体，ext→指挥
    /// 连线合法且必须生效——此前只收集成员 grants，指挥绑定静默丢失）。
    pub coordinator_grants: Vec<BoundExtension>,
    /// 可派成员：指挥态 = dispatch 边目标；并行态 = 全部成员。
    pub members: Vec<SquadMember>,
    /// 主图 Extension → Squad 的军团级授权（全员生效）。
    pub squad_grants: Vec<BoundExtension>,
}

/// 按 `squads` 键解析军团编排；军团不存在 → None。
pub fn squad_plan(
    graph: &ArchitectureGraph,
    name: &str,
    plugins: &BTreeMap<String, ResolvedPlugin>,
) -> Option<SquadPlan> {
    let squad = graph.squads.get(name)?;
    let coordinator_node = squad.edges.iter().find_map(|edge| {
        (edge.source_handle.as_deref() == Some("dispatch"))
            .then(|| squad.nodes.iter().find(|node| node.id == edge.source))
            .flatten()
    });
    let coordinator_id = coordinator_node.map(|node| node.id.as_str());
    // 指挥态：dispatch 边目标即成员；并行态：全部成员。
    let member_ids: HashSet<&str> = if coordinator_id.is_some() {
        squad
            .edges
            .iter()
            .filter(|edge| edge.source_handle.as_deref() == Some("dispatch") && edge.source == coordinator_id.unwrap())
            .map(|edge| edge.target.as_str())
            .collect()
    } else {
        squad
            .nodes
            .iter()
            .filter(|node| NodeKind::parse(&node.node_type) == Some(NodeKind::Subagent))
            .map(|node| node.id.as_str())
            .collect()
    };
    let coordinator = coordinator_node.and_then(|node| node.data.agent.as_deref()).map(|agent| agent.trim().to_string());
    let members = squad
        .nodes
        .iter()
        .filter(|node| {
            member_ids.contains(node.id.as_str())
                && NodeKind::parse(&node.node_type) == Some(NodeKind::Subagent)
        })
        .map(|node| SquadMember {
            agent: node.data.agent.as_deref().unwrap_or_default().trim().to_string(),
            required: node.data.requirement != Some(ExtensionRequirement::Optional),
            grants: collect_bound(
                &squad.nodes,
                &squad.edges,
                &HashSet::from([node.id.as_str()]),
                plugins,
            ),
        })
        .collect();
    Some(SquadPlan {
        coordinator,
        coordinator_grants: coordinator_id
            .map(|id| collect_bound(&squad.nodes, &squad.edges, &HashSet::from([id]), plugins))
            .unwrap_or_default(),
        members,
        squad_grants: squad_bindings(graph, name, plugins),
    })
}

/// 多分支阻断消息合并：>1 条编号清单（cap 5 + 等 N 项），单条原样。
/// merge 语义——模型一次拿到全部未满足项，不再打地鼠。
fn merge_messages(messages: &[String]) -> String {
    if messages.len() <= 1 {
        return messages.first().cloned().unwrap_or_default();
    }
    let mut lines: Vec<String> = messages
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, m)| format!("{}. {m}", i + 1))
        .collect();
    if messages.len() > 5 {
        lines.push(format!("等 {} 项", messages.len()));
    }
    lines.join("\n")
}

/// Action 文案占位符渲染：上下文为触发该 Action 的关卡/触发器节点——
/// `{call}` 复用 describe_obligation（如「调用 load_skill ponytail-review」），
/// 换 Skill/MCP/工具后文案自动跟随，共享 Action 也各按各自上游渲染。
fn render_message(text: &str, node: &GraphNode) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let data = &node.data;
    let ext = data.extension.as_ref();
    let target = ext
        .and_then(|e| e.target.as_deref())
        .or(data.target_file.as_deref())
        .or(data.target_dir.as_deref())
        .or(data.deny_rule.as_deref())
        .unwrap_or_default();
    let tool = ext
        .and_then(|e| e.tool.as_deref())
        .or(data.tool_name.as_deref())
        .unwrap_or_default();
    let name = ext.and_then(|e| e.name.as_deref()).unwrap_or(node.id.as_str());
    let kind = match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Extension) => match ext.and_then(|e| e.kind).unwrap_or(ExtensionKind::Skill) {
            ExtensionKind::Skill => "Skill",
            ExtensionKind::Mcp => "MCP",
            ExtensionKind::Plugin => "插件",
        },
        Some(NodeKind::Requirement) => "工具",
        Some(NodeKind::Restriction) => "限制",
        Some(NodeKind::Trigger) => "触发",
        _ => "节点",
    };
    text.replace("{call}", &describe_obligation(node))
        .replace("{target}", target)
        .replace("{tool}", tool)
        .replace("{name}", name)
        .replace("{kind}", kind)
}

/// Action 空文案的自动派生：按触发它的上游关卡生成语境化默认——
/// 「留空即自动获取」，与 {call} 占位符同一思路。
fn default_gate_message(node: &GraphNode, warn: bool) -> String {
    let derived: Option<String> = match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Requirement) | Some(NodeKind::Extension) => {
            Some(format!("请先{}。", describe_obligation(node)))
        }
        Some(NodeKind::Restriction) => Some(format!(
            "违反架构限制：{}，请调整后再试。",
            node.data.deny_rule.as_deref().unwrap_or_default()
        )),
        _ => None,
    };
    match (derived, warn) {
        (Some(body), true) => format!("架构警告：{body}"),
        (Some(body), false) => body,
        (None, true) => "架构警告：该操作被拦截。".into(),
        (None, false) => "该操作被架构规则拦截。".into(),
    }
}

/// Remind 空文案的自动派生：提醒语气描述上游约束。
fn default_remind_message(node: &GraphNode) -> String {
    match NodeKind::parse(&node.node_type) {
        Some(NodeKind::Requirement) | Some(NodeKind::Extension) => {
            format!("架构提醒：与{}相关的约束仍在生效。", describe_obligation(node))
        }
        Some(NodeKind::Restriction) => format!(
            "架构提醒：注意架构限制「{}」。",
            node.data.deny_rule.as_deref().unwrap_or_default()
        ),
        _ => "架构规则提醒".into(),
    }
}

fn obligation_required(node: &GraphNode) -> bool {
    NodeKind::parse(&node.node_type) != Some(NodeKind::Extension)
        || node
            .data
            .extension
            .as_ref()
            .map(|ext| ext.is_required())
            .unwrap_or(true)
}

/// 收集图中全部有效事后关卡（Requirement/Extension 且 effective-after：
/// 显式 after + 沿 pass 边继承）。祖先 Trigger 决定何时欠账；
/// fail 分支直连的 Action 决定拦截文案。
fn collect_obligations<'a>(graph: &'a ArchitectureGraph, after: &HashSet<String>) -> Vec<Obligation<'a>> {
    let nodes = &graph.nodes;
    nodes
        .iter()
        .filter(|node| {
            matches!(
                NodeKind::parse(&node.node_type),
                Some(NodeKind::Requirement) | Some(NodeKind::Extension)
            ) && after.contains(node.id.as_str())
        })
        .map(|node| Obligation {
            node,
            triggers: trigger_ancestors(graph, &node.id),
            action: graph
                .edges
                .iter()
                .filter(|edge| {
                    edge.source == node.id
                        && edge.source_handle.as_deref().unwrap_or("fail") == "fail"
                })
                .filter_map(|edge| nodes.iter().find(|n| n.id == edge.target))
                .find(|n| NodeKind::parse(&n.node_type) == Some(NodeKind::Action)),
        })
        .collect()
}

/// Run 末欠账查询：仍挂着未验收的义务（含 optional——optional 不拦调用但会追问）。
/// kernel/agent_loop 用它决定是否在模型结束时注入强制续跑。
pub fn armed_obligations(
    graph: &ArchitectureGraph,
    history: &[HistoryEntry],
    run_id: i64,
    extensions: &ExtensionSnapshot,
    user_message: &str,
) -> Vec<ArmedObligation> {
    let dummy = PendingCall {
        tool: String::new(),
        target: String::new(),
        content: None,
    };
    let after = effective_after_set(graph);
    let eval = Eval {
        history,
        run_id,
        ext: extensions,
        by_id: HashMap::new(),
        out_edges: HashMap::new(),
        memo: HashMap::new(),
        call: &dummy,
        armed: HashSet::new(),
        after: &after,
        triggers: HashMap::new(),
        user_message,
    };
    collect_obligations(graph, &after)
        .iter()
        .filter(|obligation| eval.obligation_armed(obligation, user_message))
        .map(|obligation| ArmedObligation {
            node_id: obligation.node.id.clone(),
            message: eval.obligation_message(obligation),
        })
        .collect()
}

/// 主入口：先查事后欠账（Layer-B），再对每个命中的 Trigger 走图。
/// 返回首个产生拦截的决定性结果；trace 为已评估路径的合并（去重），前端按 node_id 高亮。
pub fn evaluate(
    graph: &ArchitectureGraph,
    call: &PendingCall,
    history: &[HistoryEntry],
    run_id: i64,
    extensions: &ExtensionSnapshot,
    user_message: &str,
) -> GraphDecision {
    let mut decision = GraphDecision::default();
    let after = effective_after_set(graph);
    let mut eval = Eval {
        history,
        run_id,
        ext: extensions,
        by_id: graph.nodes.iter().map(|node| (node.id.as_str(), node)).collect(),
        out_edges: graph.edges.iter().fold(HashMap::new(), |mut map, edge| {
            map.entry(edge.source.as_str()).or_insert_with(Vec::new).push(edge);
            map
        }),
        memo: HashMap::new(),
        call,
        armed: HashSet::new(),
        after: &after,
        triggers: graph
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    NodeKind::parse(&node.node_type),
                    Some(NodeKind::Requirement) | Some(NodeKind::Extension)
                )
            })
            .map(|node| (node.id.as_str(), trigger_ancestors(graph, &node.id)))
            .collect(),
        user_message,
    };
    let obligations = collect_obligations(graph, &after);
    let armed: Vec<&Obligation> = obligations
        .iter()
        .filter(|obligation| eval.obligation_armed(obligation, user_message))
        .collect();
    eval.armed = armed.iter().map(|obligation| obligation.node.id.as_str()).collect();

    // Layer-B 欠账门禁（optional 义务不拦调用）：满足任一欠账、豁免工具、
    // 或欠账全是 batch 且本调用同属其触发域（re-arm 累积）→ 放行；否则拦截。
    let armed_required: Vec<&&Obligation> = armed.iter().filter(|o| obligation_required(o.node)).collect();
    if !armed_required.is_empty() && !POST_EXEMPT.contains(&call.tool.as_str()) {
        let satisfies_any = armed_required
            .iter()
            .any(|o| eval.call_satisfies(o.node, call));
        let is_rearm = |o: &&&Obligation| {
            node_mode(o.node) == GateMode::Batch
                && o.triggers
                    .iter()
                    .any(|data| trigger_matches(data, call, user_message))
        };
        let blockers: Vec<&&Obligation> = armed_required.iter().filter(|o| !is_rearm(o)).copied().collect();
        if !satisfies_any && !blockers.is_empty() {
            decision.matched = true;
            decision.blocked = true;
            // merge 聚合：多笔欠账列验收清单（cap 5 + 等 N 项）；单笔沿用完整句。
            decision.message = Some(if blockers.len() == 1 {
                eval.obligation_message(blockers[0])
            } else {
                let mut lines: Vec<String> = blockers
                    .iter()
                    .take(5)
                    .enumerate()
                    .map(|(i, o)| format!("{}. {}", i + 1, eval.obligation_short(o)))
                    .collect();
                if blockers.len() > 5 {
                    lines.push(format!("等 {} 项", blockers.len()));
                }
                format!("验收未完成前不能进行其他操作，请先完成以下验收：\n{}", lines.join("\n"))
            });
            // trace：全部欠账义务亮 Pending，首笔的 Action 标 Blocked。
            for blocker in &blockers {
                decision.trace.push(TraceStep {
                    node_id: blocker.node.id.clone(),
                    status: TraceStatus::Pending,
                });
            }
            if let Some(action) = blockers[0].action {
                decision.trace.push(TraceStep {
                    node_id: action.id.clone(),
                    status: TraceStatus::Blocked,
                });
            }
            let mut seen = HashSet::new();
            decision
                .trace
                .retain(|step| seen.insert(step.node_id.clone()));
            return decision;
        }
    }

    let mut reminders: Vec<String> = Vec::new();
    for node in &graph.nodes {
        if NodeKind::parse(&node.node_type) != Some(NodeKind::Trigger) {
            continue;
        }
        if !trigger_matches(&node.data, call, user_message) {
            continue;
        }
        decision.matched = true;
        let (blocked, warn, message, mut steps) = eval.walk(&node.id);
        decision.trace.append(&mut steps);
        if blocked {
            decision.blocked = true;
            decision.warn = warn;
            decision.message = message;
            break;
        }
        // 非阻断消息仅来自 Remind 动作——归入 reminder 拼进工具结果。
        if let Some(message) = message {
            reminders.push(message);
        }
    }
    if !decision.blocked && !armed_required.is_empty() {
        // 欠账自动提醒：放行的每个调用都带上欠账清单，模型无法「忘记」验收。
        let mut items: Vec<String> = armed_required
            .iter()
            .take(3)
            .map(|o| describe_obligation(o.node))
            .collect();
        if armed_required.len() > 3 {
            items.push(format!("等 {} 项", armed_required.len()));
        }
        reminders.push(format!("【架构提醒】尚有 {} 项事后验收未完成：{}", armed_required.len(), items.join("；")));
    }
    if !reminders.is_empty() {
        decision.reminder = Some(reminders.join("\n"));
    }
    let mut seen = HashSet::new();
    decision
        .trace
        .retain(|step| seen.insert(step.node_id.clone()));
    decision
}

/// 反向 BFS：Extension 节点 ← Trigger，供架构契约描述触发上下文。
pub fn trigger_ancestors(graph: &ArchitectureGraph, node_id: &str) -> Vec<GraphNodeData> {
    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        incoming.entry(edge.target.as_str()).or_default().push(edge.source.as_str());
    }
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = vec![node_id];
    while let Some(id) = queue.pop() {
        for parent in incoming.get(id).cloned().unwrap_or_default() {
            if visited.insert(parent) {
                if let Some(node) = graph.nodes.iter().find(|node| node.id == parent) {
                    if NodeKind::parse(&node.node_type) == Some(NodeKind::Trigger) {
                        out.push(node.data.clone());
                    }
                    queue.push(parent);
                }
            }
        }
    }
    out
}

/// 图中全部必选 Extension 节点（架构契约列举用）。
pub fn required_extensions(graph: &ArchitectureGraph) -> Vec<&ExtensionData> {
    graph
        .nodes
        .iter()
        .filter(|node| NodeKind::parse(&node.node_type) == Some(NodeKind::Extension))
        .filter_map(|node| node.data.extension.as_ref())
        .filter(|ext| ext.is_required())
        .collect()
}

/// 图中全部可选 Extension 节点。
pub fn optional_extensions(graph: &ArchitectureGraph) -> Vec<&ExtensionData> {
    graph
        .nodes
        .iter()
        .filter(|node| NodeKind::parse(&node.node_type) == Some(NodeKind::Extension))
        .filter_map(|node| node.data.extension.as_ref())
        .filter(|ext| !ext.is_required())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, kind: &str, data: serde_json::Value) -> serde_json::Value {
        json!({ "id": id, "type": kind, "position": {"x":0,"y":0}, "data": data })
    }
    fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> serde_json::Value {
        json!({ "id": id, "source": source, "target": target, "sourceHandle": handle, "targetHandle": "in" })
    }
    fn graph(nodes: Vec<serde_json::Value>, edges: Vec<serde_json::Value>) -> ArchitectureGraph {
        serde_json::from_value(json!({ "version": 1, "nodes": nodes, "edges": edges })).unwrap()
    }
    fn write_call(target: &str, content: &str) -> PendingCall {
        PendingCall { tool: "write_file".into(), target: target.into(), content: Some(content.into()) }
    }
    fn skill_ext(id: &str, name: &str, required: &str) -> serde_json::Value {
        node(id, "extension", json!({ "extension": {
            "kind": "skill", "name": name, "target": name, "requirement": required
        } }))
    }
    fn history(tool: &str, target: &str, run_id: i64) -> HistoryEntry {
        HistoryEntry { tool: tool.into(), target: target.into(), run_id }
    }
    fn snapshot() -> ExtensionSnapshot {
        ExtensionSnapshot {
            skills: ["ui-guidelines".into()].into_iter().collect(),
            mcp_configured: ["pg".into()].into_iter().collect(),
            mcp_tools: [("pg".to_string(), ["query".to_string()].into_iter().collect())].into_iter().collect(),
            plugins: BTreeMap::new(),
        }
    }

    #[test]
    fn validate_rejects_cycle_and_bad_handles() {
        let cyclic = graph(
            vec![node("a", "requirement", json!({"toolName":"read_file"})), node("b", "requirement", json!({"toolName":"read_file"}))],
            vec![edge("e1", "a", "b", Some("pass")), edge("e2", "b", "a", Some("pass"))],
        );
        assert!(validate_graph(&cyclic).is_err());

        let optional_fail = graph(
            vec![skill_ext("x", "ui-guidelines", "optional"), node("a", "action", json!({"result":"block","message":"m"}))],
            vec![edge("e1", "x", "a", Some("fail"))],
        );
        assert!(validate_graph(&optional_fail).is_err());
    }

    #[test]
    fn required_skill_blocks_until_load_skill() {
        let g = graph(
            vec![
                node("t", "trigger", json!({"targetDir":"src/ui","action":"modify"})),
                skill_ext("x", "ui-guidelines", "required"),
                node("a", "action", json!({"result":"block","message":"请先加载 UI-Guidelines 技能。"})),
            ],
            vec![
                edge("e1", "t", "x", Some("out")),
                edge("e2", "x", "a", Some("fail")),
            ],
        );
        validate_graph(&g).unwrap();
        let call = write_call("src/ui/App.tsx", "body");
        let none = evaluate(&g, &call, &[], 1, &snapshot(), "");
        assert!(none.matched && none.blocked);
        assert_eq!(none.message.as_deref(), Some("请先加载 UI-Guidelines 技能。"));
        assert_eq!(none.trace.iter().map(|s| s.status).collect::<Vec<_>>(), [TraceStatus::Matched, TraceStatus::Failed, TraceStatus::Blocked]);

        let used = vec![history("load_skill", "ui-guidelines", 9)];
        let ok = evaluate(&g, &call, &used, 1, &snapshot(), "");
        assert!(ok.matched && !ok.blocked);
    }

    #[test]
    fn run_scope_requires_fresh_satisfaction() {
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"any"})),
                { let mut value = skill_ext("x", "ui-guidelines", "required"); value["data"]["extension"]["scope"] = json!("run"); value },
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        let call = write_call("src/a.ts", "x");
        let used_last_run = vec![history("load_skill", "ui-guidelines", 1)];
        // scope=run：上一轮加载不算数，本轮必须重新调用。
        assert!(evaluate(&g, &call, &used_last_run, 2, &snapshot(), "").blocked);
        let used_this_run = vec![history("load_skill", "ui-guidelines", 2)];
        assert!(!evaluate(&g, &call, &used_this_run, 2, &snapshot(), "").blocked);
    }

    #[test]
    fn mcp_and_plugin_requirements() {
        let mut snap = snapshot();
        snap.plugins.insert("pg-check".into(), ResolvedPlugin { kind: ExtensionKind::Mcp, target: "pg".into(), tool: Some("query".into()) });
        let mcp_ext = |id| node(id, "extension", json!({ "extension": {"kind":"mcp","name":"pg","target":"pg","tool":"query","requirement":"required"} }));
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                mcp_ext("x"),
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        let call = write_call("src/a.ts", "x");
        assert!(evaluate(&g, &call, &[], 1, &snap, "").blocked);
        let wrong_tool = vec![history("mcp__pg__other", "{}", 1)];
        assert!(evaluate(&g, &call, &wrong_tool, 1, &snap, "").blocked);
        let used = vec![history("mcp__pg__query", "{}", 1)];
        assert!(!evaluate(&g, &call, &used, 1, &snap, "").blocked);

        // 插件别名到同一 MCP 工具。
        let gp = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("x", "extension", json!({ "extension": {"kind":"plugin","name":"pg-check","target":"pg-check","requirement":"required"} })),
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        assert!(!evaluate(&gp, &call, &used, 1, &snap, "").blocked);
    }

    #[test]
    fn read_command_plan_triggers() {
        let g = graph(
            vec![
                node("tr", "trigger", json!({"action":"read","targetDir":"src/db"})),
                node("tc", "trigger", json!({"action":"command","targetDir":"rm -rf"})),
                node("tp", "trigger", json!({"action":"plan"})),
                node("a", "action", json!({"result":"block","message":"stop"})),
            ],
            vec![
                edge("e1", "tr", "a", Some("out")),
                edge("e2", "tc", "a", Some("out")),
                edge("e3", "tp", "a", Some("out")),
            ],
        );
        let read_hit = PendingCall { tool: "read_file".into(), target: "src/db/schema.sql".into(), content: None };
        assert!(evaluate(&g, &read_hit, &[], 1, &snapshot(), "").blocked);
        let read_miss = PendingCall { tool: "read_file".into(), target: "src/ui/a.ts".into(), content: None };
        assert!(!evaluate(&g, &read_miss, &[], 1, &snapshot(), "").blocked);
        let cmd = PendingCall { tool: "run_command".into(), target: "rm -rf x".into(), content: Some("rm -rf x".into()) };
        assert!(evaluate(&g, &cmd, &[], 1, &snapshot(), "").blocked);
        let plan = PendingCall { tool: "update_plan".into(), target: "1/2".into(), content: None };
        assert!(evaluate(&g, &plan, &[], 1, &snapshot(), "").blocked);
        let grep = PendingCall { tool: "grep_files".into(), target: "foo".into(), content: None };
        assert!(!evaluate(&g, &grep, &[], 1, &snapshot(), "").matched);
    }

    #[test]
    fn restriction_checks_write_content() {
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src/ui"})),
                node("r", "restriction", json!({"denyRule":"src/db"})),
                node("a", "action", json!({"result":"block","message":"不允许在 UI 层引入 DB 层，请重写。"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "a", Some("fail")),
            ],
        );
        let dirty = write_call("src/ui/App.tsx", "import db from '../src/db/x'");
        assert!(evaluate(&g, &dirty, &[], 1, &snapshot(), "").blocked);
        let windows = write_call("src/ui/App.tsx", r#"import db from 'src\db\x'"#);
        assert!(evaluate(&g, &windows, &[], 1, &snapshot(), "").blocked);
        let clean = write_call("src/ui/App.tsx", "import x from 'y'");
        assert!(!evaluate(&g, &clean, &[], 1, &snapshot(), "").blocked);
    }

    #[test]
    fn prompt_trigger_matches_user_message() {
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"prompt","targetDir":"重构"})),
                node("r", "requirement", json!({"toolName":"spawn_subagent","scope":"run"})),
                node("a", "action", json!({"result":"block","message":"先拆分调研"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "a", Some("fail")),
            ],
        );
        let call = write_call("src/lib.rs", "x");
        // 消息命中关键词 → 要求未满足时拦截；消息不含关键词 → 链不生效。
        assert!(evaluate(&g, &call, &[], 1, &snapshot(), "请先重构 src/lib.rs").blocked);
        assert!(!evaluate(&g, &call, &[], 1, &snapshot(), "修复一个错别字").matched);
        // 空关键词 = 任意用户输入都命中。
        let g_any = graph(
            vec![
                node("t", "trigger", json!({"action":"prompt"})),
                node("a", "action", json!({"result":"block","message":"stop"})),
            ],
            vec![edge("e1", "t", "a", Some("out"))],
        );
        assert!(evaluate(&g_any, &call, &[], 1, &snapshot(), "任意消息").blocked);
        // 关键词大小写不敏感。
        let g_case = graph(
            vec![
                node("t", "trigger", json!({"action":"prompt","targetDir":"Refactor"})),
                node("a", "action", json!({"result":"block","message":"stop"})),
            ],
            vec![edge("e1", "t", "a", Some("out"))],
        );
        assert!(evaluate(&g_case, &call, &[], 1, &snapshot(), "please REFACTOR this").blocked);
    }

    // ---- 事后验收（postcondition） ------------------------------------------------

    fn skill_ext_after(id: &str, name: &str, mode: Option<&str>) -> serde_json::Value {
        let mut value = node(id, "extension", json!({ "extension": {
            "kind": "skill", "name": name, "target": name,
            "requirement": "required", "timing": "after"
        } }));
        if let Some(mode) = mode {
            value["data"]["extension"]["mode"] = json!(mode);
        }
        value
    }

    fn command_call(command: &str) -> PendingCall {
        PendingCall { tool: "run_command".into(), target: command.into(), content: Some(command.into()) }
    }

    fn postcondition_graph(mode: Option<&str>) -> ArchitectureGraph {
        graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                skill_ext_after("x", "ui-review", mode),
                node("a", "action", json!({"result":"block","message":"请先调用 ui-review 完成代码审查。"})),
            ],
            vec![
                edge("e1", "t", "x", Some("out")),
                edge("e2", "x", "a", Some("fail")),
            ],
        )
    }

    fn review_snapshot() -> ExtensionSnapshot {
        ExtensionSnapshot {
            skills: ["ui-review".into()].into_iter().collect(),
            ..snapshot()
        }
    }

    #[test]
    fn postcondition_batch_arms_blocks_and_disarms() {
        let g = postcondition_graph(None);
        validate_graph(&g).unwrap();
        let snap = review_snapshot();
        let write1 = vec![history("write_file", "src/a.ts", 1)];

        // 首笔触发调用：欠账从它开始，本笔放行。
        let call = write_call("src/a.ts", "x");
        assert!(!evaluate(&g, &call, &[], 1, &snap, "").blocked);
        // batch：欠账期间同类触发放行累积（第二次写仍放行）。
        assert!(!evaluate(&g, &call, &write1, 1, &snap, "").blocked);
        // 非豁免调用被拦：fail 分支 Action 的文案。
        let blocked = evaluate(&g, &command_call("cargo test"), &write1, 1, &snap, "");
        assert!(blocked.matched && blocked.blocked);
        assert_eq!(blocked.message.as_deref(), Some("请先调用 ui-review 完成代码审查。"));
        assert_eq!(
            blocked.trace.iter().map(|s| s.status).collect::<Vec<_>>(),
            [TraceStatus::Pending, TraceStatus::Blocked]
        );
        // 豁免集放行（读/计划/提问/load_skill 不拦）。
        let read = PendingCall { tool: "read_file".into(), target: "src/a.ts".into(), content: None };
        assert!(!evaluate(&g, &read, &write1, 1, &snap, "").blocked);
        // 满足调用放行。
        let satisfy = PendingCall { tool: "load_skill".into(), target: "ui-review".into(), content: None };
        assert!(!evaluate(&g, &satisfy, &write1, 1, &snap, "").blocked);
        // 还账后一切恢复。
        let done = vec![history("write_file", "src/a.ts", 1), history("load_skill", "ui-review", 1)];
        assert!(!evaluate(&g, &command_call("cargo test"), &done, 1, &snap, "").blocked);
        // 再写一次 → 重新欠账。
        let again = vec![history("write_file", "src/a.ts", 1), history("load_skill", "ui-review", 1), history("write_file", "src/b.ts", 1)];
        assert!(evaluate(&g, &command_call("cargo test"), &again, 1, &snap, "").blocked);
        // Run 末欠账查询返回该义务。
        let outstanding = armed_obligations(&g, &write1, 1, &snap, "");
        assert_eq!(outstanding.len(), 1);
        assert_eq!(outstanding[0].message, "请先调用 ui-review 完成代码审查。");
    }

    #[test]
    fn postcondition_strict_blocks_repeat_trigger() {
        let g = postcondition_graph(Some("strict"));
        validate_graph(&g).unwrap();
        let snap = review_snapshot();
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        // strict：欠账期间同类触发调用也拦（改一次必须审一次）。
        let blocked = evaluate(&g, &write_call("src/b.ts", "x"), &write1, 1, &snap, "");
        assert!(blocked.blocked);
        assert_eq!(blocked.message.as_deref(), Some("请先调用 ui-review 完成代码审查。"));
        // 满足调用仍然放行。
        let satisfy = PendingCall { tool: "load_skill".into(), target: "ui-review".into(), content: None };
        assert!(!evaluate(&g, &satisfy, &write1, 1, &snap, "").blocked);
    }

    #[test]
    fn postcondition_optional_and_missing_extension() {
        // optional：欠账进欠账列表但不拦截调用。
        let opt = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                { let mut v = skill_ext_after("x", "ui-review", None); v["data"]["extension"]["requirement"] = json!("optional"); v },
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        let snap = review_snapshot();
        assert!(!evaluate(&opt, &command_call("cargo test"), &write1, 1, &snap, "").blocked);
        assert_eq!(armed_obligations(&opt, &write1, 1, &snap, "").len(), 1);

        // Skill 未安装：欠账还不掉 → 触发调用走 fail 分支给 Action 文案；
        // 非豁免调用在 Layer-B 拦下并提示缺失原因。
        let g = postcondition_graph(None);
        let empty = ExtensionSnapshot::default();
        let blocked = evaluate(&g, &write_call("src/b.ts", "x"), &write1, 1, &empty, "");
        assert!(blocked.blocked && blocked.message.as_deref().unwrap().contains("ui-review"));
        let blocked_cmd = evaluate(&g, &command_call("ls"), &write1, 1, &empty, "");
        assert!(blocked_cmd.blocked && blocked_cmd.message.as_deref().unwrap().contains("未安装"));
    }

    #[test]
    fn postcondition_satisfying_call_never_self_arms() {
        // any 触发 + 事后验收：满足调用自身不欠账（否则永远还不清）。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"any"})),
                node("r", "requirement", json!({"toolName":"run_command","timing":"after"})),
            ],
            vec![edge("e1", "t", "r", Some("out"))],
        );
        let snap = snapshot();
        // 历史里只有满足调用本身 → 不欠账。
        let only_cmd = vec![history("run_command", "cargo test", 1)];
        assert!(!evaluate(&g, &write_call("src/a.ts", "x"), &only_cmd, 1, &snap, "").blocked);
        // write 欠账后 run_command 满足放行。
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        assert!(!evaluate(&g, &command_call("cargo test"), &write1, 1, &snap, "").blocked);
        // write + run_command（既是触发又是满足）→ 还账完成，后续放行。
        let done = vec![history("write_file", "src/a.ts", 1), history("run_command", "cargo test", 1)];
        assert!(!evaluate(&g, &command_call("cargo fmt"), &done, 1, &snap, "").blocked);
    }

    #[test]
    fn postcondition_run_scope_resets_per_run() {
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                { let mut v = skill_ext_after("x", "ui-review", None); v["data"]["extension"]["scope"] = json!("run"); v },
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        let snap = review_snapshot();
        // 上一轮的欠账记录对本轮不可见：新 run 里还没触发 → 不欠账。
        let last_run = vec![history("write_file", "src/a.ts", 1)];
        assert!(!evaluate(&g, &command_call("cargo test"), &last_run, 2, &snap, "").blocked);
        // 本轮触发 → 本轮欠账。
        let this_run = vec![history("write_file", "src/a.ts", 2)];
        assert!(evaluate(&g, &command_call("cargo test"), &this_run, 2, &snap, "").blocked);
    }

    #[test]
    fn postcondition_requirement_after_tool() {
        // 普通 Requirement 也能做事后：改码后必须先拆分评审。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("r", "requirement", json!({"toolName":"spawn_subagent","timing":"after"})),
                node("a", "action", json!({"result":"block","message":"请先拆分评审。"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "a", Some("fail")),
            ],
        );
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        // batch：第二次写放行；非触发命令被拦。
        let blocked = evaluate(&g, &write_call("src/c.ts", "x"), &write1, 1, &snapshot(), "");
        assert!(!blocked.blocked);
        assert_eq!(
            evaluate(&g, &command_call("git push"), &write1, 1, &snapshot(), "").message.as_deref(),
            Some("请先拆分评审。")
        );
        // 满足调用放行。
        let satisfy = PendingCall { tool: "spawn_subagent".into(), target: "评审".into(), content: None };
        assert!(!evaluate(&g, &satisfy, &write1, 1, &snapshot(), "").blocked);
    }

    // ---- 事后语义继承 + 提醒 + each scope ----------------------------------------

    #[test]
    fn postcondition_cascade_makes_downstream_skill_obligation() {
        // 用户直觉语义：事后关卡 pass 下游未设时机的 Skill 也是事后义务。
        // 否则它被当事前关卡，第一次修改本身就被拦死、欠账永远欠不上。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("r", "requirement", json!({"toolName":"run_command","timing":"after"})),
                node("s", "extension", json!({ "extension": {
                    "kind":"skill","name":"ui-review","target":"ui-review","requirement":"required"
                } })),
                node("a", "action", json!({"result":"block","message":"改码后必须先验收。"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "s", Some("pass")),
                edge("e3", "r", "a", Some("fail")),
                edge("e4", "s", "a", Some("fail")),
            ],
        );
        validate_graph(&g).unwrap();
        let set = effective_after_set(&g);
        assert!(set.contains("r") && set.contains("s"));
        let snap = review_snapshot();
        // 首个修改放行（若 Skill 被当事前，这里就会被拦）。
        assert!(!evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snap, "").blocked);
        // 修改后欠两笔：run_command + load_skill。
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        let outstanding = armed_obligations(&g, &write1, 1, &snap, "");
        assert_eq!(outstanding.len(), 2);
        // 非豁免调用被拦（欠账期间）。
        assert!(evaluate(&g, &PendingCall { tool: "spawn_subagent".into(), target: "x".into(), content: None }, &write1, 1, &snap, "").blocked);
        // 还一笔：load_skill 之后仍欠 run_command（注意 run_command 义务
        // 会被任意命令满足——拦截断言要用非命令类的实质调用）。
        let half = vec![history("write_file", "src/a.ts", 1), history("load_skill", "ui-review", 1)];
        assert_eq!(armed_obligations(&g, &half, 1, &snap, "").len(), 1);
        assert!(evaluate(&g, &PendingCall { tool: "spawn_subagent".into(), target: "x".into(), content: None }, &half, 1, &snap, "").blocked);
        // run_command 还清 → 全部解除。
        let done = vec![history("write_file", "src/a.ts", 1), history("load_skill", "ui-review", 1), history("run_command", "cargo test", 1)];
        assert!(armed_obligations(&g, &done, 1, &snap, "").is_empty());
        assert!(!evaluate(&g, &command_call("ls"), &done, 1, &snap, "").blocked);
    }

    #[test]
    fn postcondition_cascade_pinned_before_stays_before() {
        // 显式 timing=before 钉住为事前：不继承、仍按调用前判定。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("r", "requirement", json!({"toolName":"run_command","timing":"after"})),
                { let mut v = node("s", "extension", json!({ "extension": {
                    "kind":"skill","name":"ui-review","target":"ui-review","requirement":"required"
                } })); v["data"]["extension"]["timing"] = json!("before"); v },
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "s", Some("pass")),
            ],
        );
        assert!(!effective_after_set(&g).contains("s"));
        let snap = review_snapshot();
        // 事前 Skill 未加载 → 第一次修改被它拦（用户钉住事前的预期语义）。
        assert!(evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snap, "").blocked);
        let used = vec![history("load_skill", "ui-review", 1)];
        assert!(!evaluate(&g, &write_call("src/a.ts", "x"), &used, 1, &snap, "").blocked);
    }

    #[test]
    fn postcondition_cascade_multi_level() {
        // 继承沿 pass 链向下传播多级。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("r", "requirement", json!({"toolName":"run_command","timing":"after"})),
                node("s1", "extension", json!({ "extension": {
                    "kind":"skill","name":"ui-review","target":"ui-review","requirement":"required"
                } })),
                node("s2", "extension", json!({ "extension": {
                    "kind":"skill","name":"ui-guidelines","target":"ui-guidelines","requirement":"required"
                } })),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "s1", Some("pass")),
                edge("e3", "s1", "s2", Some("pass")),
            ],
        );
        let set = effective_after_set(&g);
        assert!(set.contains("r") && set.contains("s1") && set.contains("s2"));
    }

    #[test]
    fn remind_action_passes_and_attaches() {
        // Remind 动作：调用放行，消息作为 reminder 拼进工具结果（区别于 warn 的拦截）。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("a", "action", json!({"result":"remind","message":"改动须与已批准计划一致"})),
            ],
            vec![edge("e1", "t", "a", Some("out"))],
        );
        let decision = evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snapshot(), "");
        assert!(decision.matched && !decision.blocked);
        assert_eq!(decision.reminder.as_deref(), Some("改动须与已批准计划一致"));
        assert_eq!(decision.trace.last().map(|s| s.status), Some(TraceStatus::Reminded));
    }

    #[test]
    fn armed_obligation_auto_reminder_on_allowed_calls() {
        // 欠账期间放行的调用自动附欠账提醒——「执行中时刻提醒」的核心通道。
        let g = postcondition_graph(None);
        let snap = review_snapshot();
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        // 豁免读调用：放行且带欠账提醒。
        let read = PendingCall { tool: "read_file".into(), target: "src/a.ts".into(), content: None };
        let decision = evaluate(&g, &read, &write1, 1, &snap, "");
        assert!(!decision.blocked);
        let reminder = decision.reminder.unwrap();
        assert!(reminder.contains("1 项事后验收未完成") && reminder.contains("load_skill"));
        // 还账后不再提醒。
        let done = vec![history("write_file", "src/a.ts", 1), history("load_skill", "ui-review", 1)];
        assert!(evaluate(&g, &read, &done, 1, &snap, "").reminder.is_none());
    }

    #[test]
    fn each_scope_requires_fresh_satisfaction() {
        // scope=each：上次同链触发之后的调用才算数——「每次写码前重新满足」。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                { let mut v = skill_ext("x", "ui-guidelines", "required"); v["data"]["extension"]["scope"] = json!("each"); v },
            ],
            vec![edge("e1", "t", "x", Some("out"))],
        );
        validate_graph(&g).unwrap();
        let snap = snapshot();
        let call = write_call("src/a.ts", "x");
        // 加载过 → 首次放行。
        let used = vec![history("load_skill", "ui-guidelines", 1)];
        assert!(!evaluate(&g, &call, &used, 1, &snap, "").blocked);
        // 加载在触发之前 → 触发后再写必须重新加载。
        let stale = vec![history("load_skill", "ui-guidelines", 1), history("write_file", "src/a.ts", 1)];
        assert!(evaluate(&g, &call, &stale, 1, &snap, "").blocked);
        // 触发后重新加载 → 放行。
        let fresh = vec![
            history("load_skill", "ui-guidelines", 1),
            history("write_file", "src/a.ts", 1),
            history("load_skill", "ui-guidelines", 1),
        ];
        assert!(!evaluate(&g, &call, &fresh, 1, &snap, "").blocked);
        // scope=each 用在事后关卡上无意义 → validate 拒绝。
        let bad = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                { let mut v = node("r", "requirement", json!({"toolName":"run_command","timing":"after"})); v["data"]["scope"] = json!("each"); v },
            ],
            vec![edge("e1", "t", "r", Some("out"))],
        );
        assert!(validate_graph(&bad).is_err());
    }

    #[test]
    fn command_substring_satisfies_verification() {
        // 「改码后必须跑 cargo test」：命令目标按子串判定（不是路径匹配）。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("r", "requirement", json!({"toolName":"run_command","targetFile":"cargo test","timing":"after"})),
                node("a", "action", json!({"result":"block","message":"请先运行测试。"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "a", Some("fail")),
            ],
        );
        let snap = snapshot();
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        // cargo fmt 不算数（不满足验收）→ 拦截。
        assert!(evaluate(&g, &PendingCall { tool: "spawn_subagent".into(), target: "x".into(), content: None }, &write1, 1, &snap, "").blocked);
        // cargo test --lib 子串命中 → 放行还账。
        assert!(!evaluate(&g, &command_call("cargo test --lib"), &write1, 1, &snap, "").blocked);
        let done = vec![history("write_file", "src/a.ts", 1), history("run_command", "cargo test --lib", 1)];
        assert!(armed_obligations(&g, &done, 1, &snap, "").is_empty());
        // 历史里的 cargo fmt 不解除欠账。
        let fmt_only = vec![history("write_file", "src/a.ts", 1), history("run_command", "cargo fmt", 1)];
        assert_eq!(armed_obligations(&g, &fmt_only, 1, &snap, "").len(), 1);
        // 事前 Requirement 同样支持命令子串：command 触发器 + 历史中跑过测试。
        let before = graph(
            vec![
                node("t", "trigger", json!({"action":"command","targetDir":"git push"})),
                node("r", "requirement", json!({"toolName":"run_command","targetFile":"cargo test"})),
                node("a", "action", json!({"result":"block","message":"先跑测试再推送"})),
            ],
            vec![
                edge("e1", "t", "r", Some("out")),
                edge("e2", "r", "a", Some("fail")),
            ],
        );
        let push = command_call("git push");
        assert!(evaluate(&before, &push, &[], 1, &snap, "").blocked);
        assert!(evaluate(&before, &push, &fmt_only[1..], 1, &snap, "").blocked);
        assert!(!evaluate(&before, &push, &done[1..], 1, &snap, "").blocked);
    }

    #[test]
    fn action_message_placeholders_render_with_gate_node() {
        // 事后 Skill 义务 + 占位符文案：{call}/{target}/{kind} 按关卡节点实时渲染。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                skill_ext_after("x", "ponytail-review", None),
                node("a", "action", json!({"result":"block","message":"修改尚未验收：请先{call}（{target}·{kind}）完成审查。"})),
            ],
            vec![
                edge("e1", "t", "x", Some("out")),
                edge("e2", "x", "a", Some("fail")),
            ],
        );
        validate_graph(&g).unwrap();
        let snap = ExtensionSnapshot {
            skills: ["ponytail-review".into()].into_iter().collect(),
            ..snapshot()
        };
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        let blocked = evaluate(&g, &command_call("cargo test"), &write1, 1, &snap, "");
        assert!(blocked.blocked);
        assert_eq!(
            blocked.message.as_deref(),
            Some("修改尚未验收：请先调用 load_skill ponytail-review（ponytail-review·Skill）完成审查。")
        );
        // Run 末追问的欠账消息同样按占位符渲染。
        let outstanding = armed_obligations(&g, &write1, 1, &snap, "");
        assert_eq!(outstanding[0].message, blocked.message.unwrap());
    }

    #[test]
    fn action_message_placeholders_shared_action_per_gate() {
        // 共享 Action：两个 Skill 关卡 fail 连同一 Action——{call} 按各自上游渲染不串名。
        // 事前链路验证 walk_children 路径：x1 未满足时消息说 skill-a。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                node("x1", "extension", json!({"extension":{"kind":"skill","name":"skill-a","target":"skill-a","requirement":"required","scope":"session"}})),
                node("x2", "extension", json!({"extension":{"kind":"skill","name":"skill-b","target":"skill-b","requirement":"required","scope":"session"}})),
                node("a", "action", json!({"result":"block","message":"请先{call}。"})),
            ],
            vec![
                edge("e1", "t", "x1", Some("out")),
                edge("e2", "t", "x2", Some("out")),
                edge("e3", "x1", "a", Some("fail")),
                edge("e4", "x2", "a", Some("fail")),
            ],
        );
        validate_graph(&g).unwrap();
        let snap = ExtensionSnapshot {
            skills: ["skill-a".into(), "skill-b".into()].into_iter().collect(),
            ..snapshot()
        };
        let call = write_call("src/a.ts", "x");
        let blocked = evaluate(&g, &call, &[], 1, &snap, "");
        assert!(blocked.blocked);
        // merge 聚合：两个未满足关卡各自渲染占位符后合并——共享 Action 不串名。
        assert_eq!(
            blocked.message.as_deref(),
            Some("1. 请先调用 load_skill skill-a。\n2. 请先调用 load_skill skill-b。")
        );
        // skill-a 已加载、skill-b 未加载 → 单条为 skill-b 的渲染文案。
        let loaded_a = vec![history("load_skill", "skill-a", 1)];
        let blocked_b = evaluate(&g, &call, &loaded_a, 1, &snap, "");
        assert_eq!(blocked_b.message.as_deref(), Some("请先调用 load_skill skill-b。"));
        // 无占位符文案原样透传。
        let plain = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("r", "requirement", json!({"toolName":"read_file","targetFile":"docs/a.md"})),
                node("a", "action", json!({"result":"block","message":"静态文案不含占位符"})),
            ],
            vec![edge("e1", "t", "r", Some("out")), edge("e2", "r", "a", Some("fail"))],
        );
        let hit = evaluate(&plain, &call, &[], 1, &snap, "");
        assert_eq!(hit.message.as_deref(), Some("静态文案不含占位符"));
    }

    #[test]
    fn empty_action_message_derives_from_upstream_gate() {
        // 「留空自动获取」：Action 无文案时按上游关卡生成语境化默认。
        let snap = ExtensionSnapshot {
            skills: ["ui-review".into()].into_iter().collect(),
            ..snapshot()
        };
        let call = write_call("src/a.ts", "x");

        // Skill 关卡 fail → 空 Action：默认「请先调用 load_skill ui-review。」
        let skill_graph = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                skill_ext("x", "ui-review", "required"),
                node("a", "action", json!({"result":"block"})),
            ],
            vec![edge("e1", "t", "x", Some("out")), edge("e2", "x", "a", Some("fail"))],
        );
        let hit = evaluate(&skill_graph, &call, &[], 1, &snap, "");
        assert_eq!(hit.message.as_deref(), Some("请先调用 load_skill ui-review。"));

        // Restriction fail → 空 Action：默认违反限制文案。
        let restriction_graph = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("r", "restriction", json!({"denyRule":"src/db"})),
                node("a", "action", json!({"result":"block"})),
            ],
            vec![edge("e1", "t", "r", Some("out")), edge("e2", "r", "a", Some("fail"))],
        );
        let deny = PendingCall { tool: "write_file".into(), target: "src/ui/x.ts".into(), content: Some("import db from 'src/db'".into()) };
        let hit = evaluate(&restriction_graph, &deny, &[], 1, &snap, "");
        assert_eq!(hit.message.as_deref(), Some("违反架构限制：src/db，请调整后再试。"));

        // Trigger out → 空 Action 直连：通用拦截句。
        let direct = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("a", "action", json!({"result":"block"})),
            ],
            vec![edge("e1", "t", "a", Some("out"))],
        );
        let hit = evaluate(&direct, &call, &[], 1, &snap, "");
        assert_eq!(hit.message.as_deref(), Some("该操作被架构规则拦截。"));

        // warn 变体：「架构警告：」前缀。
        let warn_graph = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                skill_ext("x", "ui-review", "required"),
                node("a", "action", json!({"result":"warn"})),
            ],
            vec![edge("e1", "t", "x", Some("out")), edge("e2", "x", "a", Some("fail"))],
        );
        let hit = evaluate(&warn_graph, &call, &[], 1, &snap, "");
        assert!(hit.warn);
        assert_eq!(hit.message.as_deref(), Some("架构警告：请先调用 load_skill ui-review。"));
    }

    #[test]
    fn empty_remind_message_derives_reminder() {
        // Remind 空文案：提醒语气描述上游约束；Trigger 直连给通用句。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("a", "action", json!({"result":"remind"})),
            ],
            vec![edge("e1", "t", "a", Some("out"))],
        );
        let call = write_call("src/a.ts", "x");
        let decision = evaluate(&g, &call, &[], 1, &snapshot(), "");
        assert!(!decision.blocked);
        assert_eq!(decision.reminder.as_deref(), Some("架构规则提醒"));
    }

    #[test]
    fn merge_multi_branch_blockers_into_numbered_list() {
        // merge：两个 Skill 关卡 fail 汇入同一空 Action——拦截提示逐条列出全部未满足项。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("x1", "extension", json!({"extension":{"kind":"skill","name":"skill-a","target":"skill-a","requirement":"required"}})),
                node("x2", "extension", json!({"extension":{"kind":"skill","name":"skill-b","target":"skill-b","requirement":"required"}})),
                node("a", "action", json!({"result":"block"})),
            ],
            vec![
                edge("e1", "t", "x1", Some("out")),
                edge("e2", "t", "x2", Some("out")),
                edge("e3", "x1", "a", Some("fail")),
                edge("e4", "x2", "a", Some("fail")),
            ],
        );
        validate_graph(&g).unwrap();
        let snap = ExtensionSnapshot {
            skills: ["skill-a".into(), "skill-b".into()].into_iter().collect(),
            ..snapshot()
        };
        let hit = evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snap, "");
        assert!(hit.blocked && !hit.warn);
        assert_eq!(
            hit.message.as_deref(),
            Some("1. 请先调用 load_skill skill-a。\n2. 请先调用 load_skill skill-b。")
        );
        // 部分满足：仍合并列出剩余项。
        let loaded_a = vec![history("load_skill", "skill-a", 1)];
        let hit2 = evaluate(&g, &write_call("src/a.ts", "x"), &loaded_a, 1, &snap, "");
        assert_eq!(hit2.message.as_deref(), Some("请先调用 load_skill skill-b。"));
    }

    #[test]
    fn merge_warn_and_block_becomes_block() {
        // 一条 warn 分支 + 一条 block 分支同时失败 → 整体 block，消息合并。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify"})),
                node("r1", "requirement", json!({"toolName":"read_file","targetFile":"docs/a.md"})),
                node("r2", "requirement", json!({"toolName":"read_file","targetFile":"docs/b.md"})),
                node("aw", "action", json!({"result":"warn","message":"警告：建议读 a"})),
                node("ab", "action", json!({"result":"block","message":"必须读 b"})),
            ],
            vec![
                edge("e1", "t", "r1", Some("out")),
                edge("e2", "t", "r2", Some("out")),
                edge("e3", "r1", "aw", Some("fail")),
                edge("e4", "r2", "ab", Some("fail")),
            ],
        );
        let hit = evaluate(&g, &write_call("src/x.ts", "x"), &[], 1, &snapshot(), "");
        assert!(hit.blocked && !hit.warn);
        assert_eq!(hit.message.as_deref(), Some("1. 警告：建议读 a\n2. 必须读 b"));
    }

    #[test]
    fn merge_outstanding_obligations_into_checklist() {
        // Layer-B：两笔欠账同时挂起 → 验收清单文案 + 全部义务亮 Pending。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                skill_ext_after("x1", "skill-a", None),
                skill_ext_after("x2", "skill-b", None),
            ],
            vec![
                edge("e1", "t", "x1", Some("out")),
                edge("e2", "t", "x2", Some("out")),
            ],
        );
        validate_graph(&g).unwrap();
        let snap = ExtensionSnapshot {
            skills: ["skill-a".into(), "skill-b".into()].into_iter().collect(),
            ..snapshot()
        };
        let write1 = vec![history("write_file", "src/a.ts", 1)];
        let hit = evaluate(&g, &command_call("cargo test"), &write1, 1, &snap, "");
        assert!(hit.blocked);
        assert_eq!(
            hit.message.as_deref(),
            Some("验收未完成前不能进行其他操作，请先完成以下验收：\n1. 调用 load_skill skill-a\n2. 调用 load_skill skill-b")
        );
        let pending: Vec<_> = hit.trace.iter().filter(|s| s.status == TraceStatus::Pending).map(|s| s.node_id.as_str()).collect();
        assert_eq!(pending, ["x1", "x2"]);
    }

    #[test]
    fn merge_messages_caps_at_five() {
        let six: Vec<String> = (1..=6).map(|i| format!("第{i}项")).collect();
        let merged = merge_messages(&six);
        assert!(merged.contains("5. 第5项") && merged.contains("等 6 项") && !merged.contains("第6项"));
        assert_eq!(merge_messages(&["独苗".into()]), "独苗");
        assert_eq!(merge_messages(&[]), "");
    }

    fn subagent_node(id: &str, agent: &str) -> serde_json::Value {
        node(id, "subagent", json!({ "agent": agent }))
    }

    #[test]
    fn subagent_node_validates_agent_and_edges() {
        // 缺 agent 名 → 拒
        let bad = graph(vec![node("s", "subagent", json!({}))], vec![]);
        assert!(validate_graph(&bad).is_err());
        // 非扩展入边（trigger → subagent）→ 拒
        let bad_in = graph(
            vec![node("t", "trigger", json!({"action":"any"})), subagent_node("s", "explore")],
            vec![edge("e1", "t", "s", Some("out"))],
        );
        assert!(validate_graph(&bad_in).is_err());
        // subagent 出边 → 拒
        let bad_out = graph(
            vec![subagent_node("s", "explore"), node("a", "action", json!({"result":"block"}))],
            vec![edge("e1", "s", "a", Some("pass"))],
        );
        assert!(validate_graph(&bad_out).is_err());
        // fail 出口接 subagent → 拒
        let bad_fail = graph(
            vec![skill_ext("x", "ui-guidelines", "required"), subagent_node("s", "explore")],
            vec![edge("e1", "x", "s", Some("fail"))],
        );
        assert!(validate_graph(&bad_fail).is_err());
        // 合法：extension --pass--> subagent
        let ok = graph(
            vec![skill_ext("x", "ui-guidelines", "required"), subagent_node("s", "explore")],
            vec![edge("e1", "x", "s", Some("pass"))],
        );
        assert!(validate_graph(&ok).is_ok());
    }

    #[test]
    fn subagent_bindings_collects_and_dedupes() {
        let g = graph(
            vec![
                skill_ext("x1", "ui-guidelines", "required"),
                skill_ext("x2", "ui-guidelines", "optional"), // 同名 optional → required 胜出
                skill_ext("x3", "plan-review", "optional"),
                node("m", "extension", json!({"extension":{"kind":"mcp","name":"pg","target":"pg","requirement":"required"}})),
                subagent_node("s", "explore"),
                subagent_node("other", "general"),
            ],
            vec![
                edge("e1", "x1", "s", Some("pass")),
                edge("e2", "x2", "s", None), // 缺省出口按 pass 计
                edge("e3", "x3", "s", Some("pass")),
                edge("e4", "m", "s", Some("pass")),
                edge("e5", "x3", "other", Some("pass")),
            ],
        );
        let bindings = subagent_bindings(&g, "explore", &BTreeMap::new());
        assert_eq!(bindings.len(), 3);
        let ui = bindings.iter().find(|b| b.target == "ui-guidelines").unwrap();
        assert!(ui.required, "同名 optional 绑定应合并为 required");
        assert!(bindings.iter().any(|b| !b.required && b.target == "plan-review"));
        assert!(bindings.iter().any(|b| b.kind == ExtensionKind::Mcp && b.target == "pg"));
        // 其它类型的绑定互不影响
        assert_eq!(subagent_bindings(&g, "general", &BTreeMap::new()).len(), 1);
        assert!(subagent_bindings(&g, "nobody", &BTreeMap::new()).is_empty());
    }

    #[test]
    fn subagent_bindings_expands_plugins() {
        let g = graph(
            vec![
                node("p", "extension", json!({"extension":{"kind":"plugin","name":"bundle","target":"bundle","requirement":"required"}})),
                node("q", "extension", json!({"extension":{"kind":"plugin","name":"ghost","target":"ghost","requirement":"required"}})),
                subagent_node("s", "explore"),
            ],
            vec![
                edge("e1", "p", "s", Some("pass")),
                edge("e2", "q", "s", Some("pass")),
            ],
        );
        let plugins: BTreeMap<String, ResolvedPlugin> = [(
            "bundle".to_string(),
            ResolvedPlugin { kind: ExtensionKind::Mcp, target: "pg".into(), tool: Some("query".into()) },
        )].into_iter().collect();
        let bindings = subagent_bindings(&g, "explore", &plugins);
        assert_eq!(bindings.len(), 2);
        let mcp = bindings.iter().find(|b| b.kind == ExtensionKind::Mcp).unwrap();
        assert_eq!((mcp.target.as_str(), mcp.tool.as_deref()), ("pg", Some("query")));
        let missing = bindings.iter().find(|b| b.kind == ExtensionKind::Plugin).unwrap();
        assert!(missing.missing.is_some());
    }

    #[test]
    fn subagent_nodes_do_not_affect_eval() {
        // 装配节点不参与求值：触发链与无装配图的判定一致。
        let g = graph(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                skill_ext("x", "ui-guidelines", "required"),
                subagent_node("s", "explore"),
            ],
            vec![
                edge("e1", "t", "x", Some("out")),
                edge("e2", "x", "s", Some("pass")),
            ],
        );
        let snap = snapshot();
        // Skill 未调用 → fail 无 Action → 默认拦截文案。
        let hit = evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snap, "");
        assert!(hit.blocked);
        // Skill 已调用 → pass 无子节点 → 放行（装配边不产生拦截）。
        let pass = evaluate(
            &g,
            &write_call("src/a.ts", "x"),
            &[history("load_skill", "ui-guidelines", 1)],
            1,
            &snap,
            "",
        );
        assert!(!pass.blocked);
    }

    // ---- 智能体军团（squad / subnet 子沙盘）----

    fn squad_node(id: &str, squad: &str) -> serde_json::Value {
        node(id, "squad", json!({ "squad": squad }))
    }

    /// 军团内部成员节点；required=false 时加 "requirement":"optional"。
    fn member_node(id: &str, agent: &str, required: bool) -> serde_json::Value {
        if required {
            subagent_node(id, agent)
        } else {
            node(id, "subagent", json!({ "agent": agent, "requirement": "optional" }))
        }
    }

    fn squad_json(nodes: Vec<serde_json::Value>, edges: Vec<serde_json::Value>) -> serde_json::Value {
        json!({ "nodes": nodes, "edges": edges })
    }

    fn graph_with_squads(
        nodes: Vec<serde_json::Value>,
        edges: Vec<serde_json::Value>,
        squads: serde_json::Value,
    ) -> ArchitectureGraph {
        serde_json::from_value(json!({ "version": 1, "nodes": nodes, "edges": edges, "squads": squads }))
            .unwrap()
    }

    #[test]
    fn squads_field_defaults_empty_for_old_files() {
        // 旧版 graph_rules.json 没有 squads 键 → 反序列化为空表，不报错。
        let g = graph(vec![], vec![]);
        assert!(g.squads.is_empty());
    }

    #[test]
    fn squad_node_requires_existing_squad_and_unique_name() {
        let inner = squad_json(vec![member_node("m", "explore", true)], vec![]);
        // 引用不存在的军团 → 拒
        let missing = graph_with_squads(
            vec![squad_node("sq", "nope")],
            vec![],
            json!({ "team": inner.clone() }),
        );
        assert!(validate_graph(&missing).is_err());
        // 两个 squad 节点同名 → 拒
        let dup = graph_with_squads(
            vec![squad_node("sq1", "team"), squad_node("sq2", "team")],
            vec![],
            json!({ "team": inner.clone() }),
        );
        assert!(validate_graph(&dup).is_err());
        // 合法：squad 节点 + 定义存在
        let ok = graph_with_squads(
            vec![squad_node("sq", "team")],
            vec![],
            json!({ "team": inner }),
        );
        assert!(validate_graph(&ok).is_ok());
        // squad 节点出边 → 拒（与 subagent 同为装配终点）
        let out = graph_with_squads(
            vec![squad_node("sq", "team"), node("a", "action", json!({"result":"block"}))],
            vec![edge("e1", "sq", "a", Some("pass"))],
            json!({ "team": squad_json(vec![member_node("m", "explore", true)], vec![]) }),
        );
        assert!(validate_graph(&out).is_err());
        // 非扩展入边（trigger → squad）→ 拒
        let bad_in = graph_with_squads(
            vec![node("t", "trigger", json!({"action":"any"})), squad_node("sq", "team")],
            vec![edge("e1", "t", "sq", Some("out"))],
            json!({ "team": squad_json(vec![member_node("m", "explore", true)], vec![]) }),
        );
        assert!(validate_graph(&bad_in).is_err());
    }

    #[test]
    fn squad_internal_validation() {
        let sq = |nodes, edges| {
            graph_with_squads(
                vec![squad_node("sq", "team")],
                vec![],
                json!({ "team": squad_json(nodes, edges) }),
            )
        };
        // 非 子智能体/扩展 节点 → 拒
        assert!(validate_graph(&sq(
            vec![member_node("m", "explore", true), node("t", "trigger", json!({"action":"any"}))],
            vec![],
        ))
        .is_err());
        // 无成员 → 拒
        assert!(validate_graph(&sq(
            vec![skill_ext("x", "ui-guidelines", "required")],
            vec![],
        ))
        .is_err());
        // 成员普通出边（非 dispatch）→ 拒
        assert!(validate_graph(&sq(
            vec![member_node("c", "cto", true), member_node("m", "explore", true)],
            vec![edge("e1", "c", "m", Some("pass"))],
        ))
        .is_err());
        // 双指挥 → 拒
        assert!(validate_graph(&sq(
            vec![
                member_node("c1", "cto", true),
                member_node("c2", "lead", true),
                member_node("m", "explore", true),
            ],
            vec![
                edge("e1", "c1", "m", Some("dispatch")),
                edge("e2", "c2", "m", Some("dispatch")),
            ],
        ))
        .is_err());
        // 指挥被 dispatch 指向 → 拒
        assert!(validate_graph(&sq(
            vec![member_node("c", "cto", true), member_node("m", "explore", true)],
            vec![edge("e1", "m", "c", Some("dispatch")), edge("e2", "c", "m", Some("dispatch"))],
        ))
        .is_err());
        // 扩展 fail 出口 → 拒
        assert!(validate_graph(&sq(
            vec![skill_ext("x", "ui-guidelines", "required"), member_node("m", "explore", true)],
            vec![edge("e1", "x", "m", Some("fail"))],
        ))
        .is_err());
        // 扩展 → 非成员（此处只能指向成员）→ 拒：ext→ext 不行
        assert!(validate_graph(&sq(
            vec![
                skill_ext("x1", "ui-guidelines", "required"),
                skill_ext("x2", "plan-review", "required"),
                member_node("m", "explore", true),
            ],
            vec![edge("e1", "x1", "x2", Some("pass"))],
        ))
        .is_err());
        // 成员再派成员 = 第二指挥 → 拒（成员环在结构上必然多指挥，DAG 检测为防御性兜底）
        assert!(validate_graph(&sq(
            vec![member_node("c", "cto", true), member_node("m1", "explore", true), member_node("m2", "review", true)],
            vec![edge("e1", "c", "m1", Some("dispatch")), edge("e2", "c", "m2", Some("dispatch")), edge("e3", "m1", "m2", Some("dispatch"))],
        ))
        .is_err());
        // 合法：指挥 + 成员 + 内部装配
        assert!(validate_graph(&sq(
            vec![
                member_node("c", "cto", true),
                member_node("m", "explore", true),
                skill_ext("x", "ui-guidelines", "required"),
            ],
            vec![edge("e1", "c", "m", Some("dispatch")), edge("e2", "x", "m", Some("pass"))],
        ))
        .is_ok());
    }

    #[test]
    fn squad_plan_coordinator_mode() {
        let g = graph_with_squads(
            vec![squad_node("sq", "team")],
            vec![],
            json!({ "team": squad_json(
                vec![
                    member_node("c", "cto", true),
                    member_node("m1", "explore", true),
                    member_node("m2", "reviewer", false),
                    member_node("stray", "general", true), // 未被 dispatch 指向 → 不算成员
                    skill_ext("x", "ui-guidelines", "required"),
                ],
                vec![
                    edge("d1", "c", "m1", Some("dispatch")),
                    edge("d2", "c", "m2", Some("dispatch")),
                    edge("b1", "x", "m1", Some("pass")),
                ],
            ) }),
        );
        validate_graph(&g).unwrap();
        let plan = squad_plan(&g, "team", &BTreeMap::new()).unwrap();
        assert_eq!(plan.coordinator.as_deref(), Some("cto"));
        assert_eq!(plan.members.len(), 2);
        let m1 = plan.members.iter().find(|m| m.agent == "explore").unwrap();
        assert!(m1.required);
        assert_eq!(m1.grants.len(), 1);
        assert_eq!(m1.grants[0].target, "ui-guidelines");
        let m2 = plan.members.iter().find(|m| m.agent == "reviewer").unwrap();
        assert!(!m2.required);
        assert!(m2.grants.is_empty());
        // 未连接的成员不进 plan
        assert!(!plan.members.iter().any(|m| m.agent == "general"));
        assert!(squad_plan(&g, "nope", &BTreeMap::new()).is_none());
    }

    #[test]
    fn squad_plan_coordinator_grants() {
        // ext --pass--> 指挥：绑定进 coordinator_grants（不进成员 grants）。
        let g = graph_with_squads(
            vec![squad_node("sq", "team")],
            vec![],
            json!({ "team": squad_json(
                vec![
                    member_node("c", "cto", true),
                    member_node("m", "explore", true),
                    skill_ext("xc", "plan-review", "required"),
                    skill_ext("xm", "ui-guidelines", "required"),
                ],
                vec![
                    edge("d1", "c", "m", Some("dispatch")),
                    edge("b1", "xc", "c", Some("pass")),
                    edge("b2", "xm", "m", Some("pass")),
                ],
            ) }),
        );
        validate_graph(&g).unwrap();
        let plan = squad_plan(&g, "team", &BTreeMap::new()).unwrap();
        assert_eq!(plan.coordinator.as_deref(), Some("cto"));
        assert_eq!(plan.coordinator_grants.len(), 1);
        assert_eq!(plan.coordinator_grants[0].target, "plan-review");
        assert!(plan.coordinator_grants[0].required);
        // 成员 grants 不受指挥绑定污染。
        let member = plan.members.iter().find(|m| m.agent == "explore").unwrap();
        assert_eq!(member.grants.len(), 1);
        assert_eq!(member.grants[0].target, "ui-guidelines");
        // 并行态：无指挥 → coordinator_grants 恒空。
        let g2 = graph_with_squads(
            vec![squad_node("sq", "team")],
            vec![],
            json!({ "team": squad_json(vec![member_node("m", "explore", true)], vec![]) }),
        );
        let plan2 = squad_plan(&g2, "team", &BTreeMap::new()).unwrap();
        assert!(plan2.coordinator.is_none() && plan2.coordinator_grants.is_empty());
    }

    #[test]
    fn squad_plan_parallel_mode_and_squad_grants() {
        let g = graph_with_squads(
            vec![
                skill_ext("gx", "ui-guidelines", "required"),
                squad_node("sq", "team"),
            ],
            vec![edge("ge", "gx", "sq", Some("pass"))],
            json!({ "team": squad_json(
                vec![member_node("m1", "explore", true), member_node("m2", "reviewer", false)],
                vec![],
            ) }),
        );
        validate_graph(&g).unwrap();
        let plan = squad_plan(&g, "team", &BTreeMap::new()).unwrap();
        // 无 dispatch 边 → 并行军团：全部成员，无指挥。
        assert!(plan.coordinator.is_none());
        assert_eq!(plan.members.len(), 2);
        // 主图 Extension → Squad = 军团级授权。
        assert_eq!(plan.squad_grants.len(), 1);
        assert_eq!(plan.squad_grants[0].target, "ui-guidelines");
    }

    #[test]
    fn squad_node_eval_transparent() {
        // squad 装配节点不参与调用求值。
        let g = graph_with_squads(
            vec![
                node("t", "trigger", json!({"action":"modify","targetDir":"src"})),
                skill_ext("x", "ui-guidelines", "required"),
                squad_node("sq", "team"),
            ],
            vec![edge("e1", "t", "x", Some("out")), edge("e2", "x", "sq", Some("pass"))],
            json!({ "team": squad_json(vec![member_node("m", "explore", true)], vec![]) }),
        );
        let snap = snapshot();
        let hit = evaluate(&g, &write_call("src/a.ts", "x"), &[], 1, &snap, "");
        assert!(hit.blocked);
        let pass = evaluate(
            &g,
            &write_call("src/a.ts", "x"),
            &[history("load_skill", "ui-guidelines", 1)],
            1,
            &snap,
            "",
        );
        assert!(!pass.blocked);
    }
}
