//! Extension-facing data contracts: skills on disk and MCP server entries.
//! Pure types only; scanning, spawning and RPC live in the adapters.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---- Skills -----------------------------------------------------------------

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

#[derive(Debug, Clone, Serialize)]
pub struct SkillImportOutcome {
    pub name: String,
    pub status: String,
    pub error: Option<String>,
}

/// A `/<name>` prompt template stored as Markdown under `commands/`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub source: SkillSource,
    pub path: String,
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

/// Trust prompt payload: the workspace carries project `.maestro/` assets
/// that stay inert until the user grants trust (per canonical path, in DB).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTrustStatus {
    pub path: String,
    /// Which asset kinds are present — `agents` / `skills` / `commands`.
    pub assets: Vec<String>,
}

// ---- MCP servers ------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
    /// Explicit "restore on startup" intent; imported servers default off.
    #[serde(default)]
    pub auto_start: bool,
    /// Explicit user opt-in for this server's `readOnlyHint`. Default false:
    /// an untrusted server's tools always count as side-effecting.
    #[serde(default)]
    pub trust: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigFile {
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolView {
    pub name: String,
    pub description: String,
    /// Standard JSON Schema from the server's `tools/list` `inputSchema`.
    pub input_schema: serde_json::Value,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub disabled: bool,
    pub auto_start: bool,
    pub trust: bool,
    pub running: bool,
    pub tool_count: usize,
    /// 运行中 Server 的 `tools/list` 结果；未运行或失败时为空。
    #[serde(default)]
    pub tools: Vec<McpToolView>,
    pub error: Option<String>,
}

// ---- 声明式插件清单（无独立插件运行时） -----------------------------------------
//
// `~/.maestro/plugins/<name>.json` 与 `<workspace>/.maestro/plugins/<name>.json`
// 声明一个「插件 = 对 Skill 或 MCP 的封装引用」；架构沙盘把插件节点解析为
// 其声明的 Skill/MCP 依赖再强制调用。清单本身不可执行，绝不伪造运行时代码。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginCondition {
    Required,
    Optional,
}

impl Default for PluginCondition {
    fn default() -> Self {
        Self::Required
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `skill` | `mcp` —— 插件声明的底层依赖类型。
    pub kind: PluginKind,
    /// skill 名或 MCP server 名。
    pub target: String,
    /// 仅 `kind: mcp` 时可用：限定具体工具。
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub condition: PluginCondition,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Skill,
    Mcp,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub name: String,
    pub description: String,
    /// 清单解析失败时为 `null`，前端以红色徽标呈现。
    pub kind: Option<PluginKind>,
    pub target: Option<String>,
    pub tool: Option<String>,
    pub condition: PluginCondition,
    pub params: BTreeMap<String, String>,
    pub source: SkillSource,
    pub path: String,
    pub error: Option<String>,
}
