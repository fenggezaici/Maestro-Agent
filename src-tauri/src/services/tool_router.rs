//! Tool router: the single registry/dispatcher for everything the model can
//! call — built-in workspace reads, session reads, the skill loader, and
//! namespaced MCP tools. Authorization (mode policy) stays on the caller;
//! the router resolves effects and executes.
use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use tokio::sync::Mutex;

use crate::{
    adapters::{db::Database, fs},
    domain::{policy, AppResult, Mode, ToolEffect, ToolSpec},
    services::extensions::ExtensionManager,
};

const NO_WORKSPACE: &str =
    "未选择工作区：当前项目未设置目录，请在输入框下方项目菜单中点击「设置目录」";

pub struct ToolRouter {
    extensions: ExtensionManager,
    workspace: Mutex<Option<PathBuf>>,
    database: Arc<Database>,
}

impl ToolRouter {
    pub fn new(maestro_dir: PathBuf, database: Arc<Database>) -> AppResult<Self> {
        Ok(Self {
            extensions: ExtensionManager::new(maestro_dir)?,
            workspace: Mutex::new(None),
            database,
        })
    }

    pub fn extensions(&self) -> &ExtensionManager {
        &self.extensions
    }

    pub async fn set_workspace_root(&self, root: Option<PathBuf>) {
        *self.workspace.lock().await = root;
    }

    pub async fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace.lock().await.clone()
    }

    /// Whether a namespaced MCP tool declared `readOnlyHint`.
    pub async fn mcp_tool_is_read_only(&self, tool: &str) -> bool {
        self.extensions.mcp.tool_is_read_only(tool).await
    }

    /// Mode authorization for one tool call. `mcp__*` tools resolve their
    /// effect from the server's own readOnlyHint annotation.
    pub async fn authorize(&self, mode: Mode, tool: &str) -> AppResult<ToolEffect> {
        if tool.starts_with("mcp__") {
            let effect = if self.mcp_tool_is_read_only(tool).await {
                ToolEffect::ReadOnly
            } else {
                ToolEffect::ProcessExecution
            };
            policy::check_mode(mode, effect, tool)?;
            Ok(effect)
        } else {
            policy::authorize(mode, tool)
        }
    }

    /// Every tool spec offered to the model this turn: skill loader, running
    /// MCP tools, and the built-in workspace/session reads — all as standard
    /// JSON Schema `ToolSpec`s.
    pub async fn tool_specs(&self) -> Vec<ToolSpec> {
        let mut tools = self.extensions.tool_specs().await;
        let workspace_root = self.workspace.lock().await.clone();
        if let Some(spec) = tools
            .iter_mut()
            .find(|tool| tool.name == "load_skill")
        {
            let names = self
                .extensions
                .skills
                .scan(self.extensions.maestro_dir(), workspace_root.as_deref())
                .into_iter()
                .map(|skill| skill.name)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                spec.description =
                    format!("{} 当前可用技能：{}", spec.description, names.join("、"));
            }
        }
        // Read tools are always offered — execution reports "未选择工作区" if unset.
        tools.push(ToolSpec {
            name: "list_dir".into(),
            description: "列出工作区目录内容（只读）。path 为相对路径，省略表示根目录。".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        });
        tools.push(ToolSpec {
            name: "read_file".into(),
            description: "读取工作区文件内容（只读，限文本）。path 为相对路径。".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        });
        tools
    }

    /// Execute one already-authorized tool call, returning bounded text for
    /// the next conversation turn. `session_id` is the caller's own session —
    /// model arguments can never redirect `read_session` to another session.
    pub async fn execute(
        &self,
        session_id: i64,
        tool: &str,
        arguments_json: &str,
    ) -> AppResult<String> {
        let arguments: serde_json::Value =
            serde_json::from_str(arguments_json).unwrap_or_else(|_| json!({}));
        match tool {
            "load_skill" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "load_skill 需要 name 参数".to_string())?;
                self.extensions
                    .skills
                    .body(
                        name,
                        self.extensions.maestro_dir(),
                        self.workspace_root().await.as_deref(),
                    )
                    .map(|body| format!("【技能 {name}】\n\n{body}"))
            }
            "list_dir" => {
                let root = self.workspace_root().await.ok_or_else(|| NO_WORKSPACE.to_string())?;
                let rel = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
                fs::list_dir(&root, rel)
            }
            "read_file" => {
                let root = self.workspace_root().await.ok_or_else(|| NO_WORKSPACE.to_string())?;
                let rel = arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "read_file 需要 path 参数".to_string())?;
                fs::read_file(&root, rel)
            }
            "read_session" => serde_json::to_string(
                &self.database.session_detail(session_id).await?,
            )
            .map_err(|error| error.to_string()),
            "list_sessions" => serde_json::to_string(&self.database.list_sessions().await?)
                .map_err(|error| error.to_string()),
            _ if tool.starts_with("mcp__") => {
                self.extensions.mcp.call_tool(tool, arguments).await
            }
            _ => Err("未注册的工具".into()),
        }
    }
}
