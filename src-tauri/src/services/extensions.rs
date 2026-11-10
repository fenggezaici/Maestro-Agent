//! Extension subsystem service: on-disk Skills plus stdio MCP servers.
use std::path::{Path, PathBuf};

use crate::{
    adapters::{mcp::McpManager, skills::SkillStore},
    domain::AppResult,
};

/// Owns both extension stores; paths are injected at app setup so tests stay hermetic.
pub struct ExtensionManager {
    pub skills: SkillStore,
    pub mcp: McpManager,
    maestro_dir: PathBuf,
}

impl ExtensionManager {
    /// `maestro_dir` is the global `~/.maestro` root (skills/, mcp.json).
    pub fn new(maestro_dir: PathBuf) -> AppResult<Self> {
        std::fs::create_dir_all(maestro_dir.join("skills"))
            .map_err(|_| "无法创建 ~/.maestro/skills 目录".to_string())?;
        let mcp = McpManager::default();
        mcp.set_config_path(maestro_dir.join("mcp.json"));
        Ok(Self {
            skills: SkillStore::default(),
            mcp,
            maestro_dir,
        })
    }

    pub fn maestro_dir(&self) -> &Path {
        &self.maestro_dir
    }

    /// Tools offered to the model this turn: the built-in loader plus every
    /// running MCP server's tools namespaced `mcp__<server>__<tool>`.
    pub async fn tool_definitions(&self) -> Vec<serde_json::Value> {
        let mut tools = vec![serde_json::json!({
            "name": "load_skill",
            "description": "加载指定名称的技能(SKILL.md)完整指令内容。当任务匹配某个已安装技能时调用。",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "技能名称（kebab-case）" }
                },
                "required": ["name"]
            }
        })];
        tools.extend(self.mcp.tool_schemas().await);
        tools
    }
}
