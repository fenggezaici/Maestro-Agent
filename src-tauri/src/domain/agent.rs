use serde::{Deserialize, Serialize};

/// Where a definition came from. Resolution precedence is
/// builtin < global < project; the same name in a higher source wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    Builtin,
    Global,
    Project,
}

fn default_source() -> AgentSource {
    AgentSource::Builtin
}

/// 批量操作的目标项（scope+name）；set_hidden 时以 effective() 的
/// def.source 定位实际文件，scope 仅作前端卡片身份标识。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentItemRef {
    pub scope: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub steps: Option<u32>,
    /// Tool-call budget per subagent run; `None`（且全局默认也为空）= 无硬顶，
    /// 子循环跑到模型自然收尾——显式设置只是安全阀。
    #[serde(default)]
    pub tool_budget: Option<u32>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "default_source")]
    pub source: AgentSource,
}

fn default_mode() -> String { "subagent".into() }

/// Upper bound applied to a configured budget to catch absurd values.
pub const MAX_SUBAGENT_TOOL_BUDGET: u32 = 1000;

/// Global defaults for every subagent field; a `None` field on an
/// `AgentDefinition` falls back here before the built-in constant. Stored at
/// `~/.maestro/agents.json` — the settings UI edits it so imported agent packs
/// need no per-agent tuning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubagentDefaults {
    /// Falls back to the session model when unset.
    pub model: Option<String>,
    pub steps: Option<u32>,
    pub tool_budget: Option<u32>,
    /// Per-round parallel spawns (clamped 1-4 at resolution).
    pub max_parallel: Option<u32>,
    /// Total spawn allowance per run (clamped 1-16 at resolution).
    pub max_total: Option<u32>,
}

impl AgentDefinition {
    pub fn builtin(name: &str) -> Option<Self> {
        match name {
            "explore" => Some(Self {
                name: "explore".into(),
                description: "只读代码调研：并行查找文件、证据路径与风险".into(),
                category: Some("研究".into()), mode: "subagent".into(), model: None,
                prompt: "你是只读代码调研子智能体。工作流程：第一轮优先用 repo_map 获取仓库符号全貌（陌生代码库），按需用 glob_files/grep_files 精确定位，然后每轮批量并行 read_file 多个关键文件；不写文件、不执行命令；输出结论、证据路径和风险。".into(),
                tools: vec![
                    "read_file".into(),
                    "list_dir".into(),
                    "grep_files".into(),
                    "glob_files".into(),
                    "repo_map".into(),
                    "run_command".into(),
                    "load_skill".into(),
                ],
                steps: None, tool_budget: None, hidden: false, color: Some("#8B5CF6".into()), source: AgentSource::Builtin,
            }),
            "general" => Some(Self {
                name: "general".into(),
                description: "只读分析总结：整理多来源信息并给出结论".into(),
                category: Some("研究".into()), mode: "subagent".into(), model: None,
                prompt: "你是只读分析子智能体。工作流程：每轮批量并行读取多个关键文件，避免逐文件顺序读取；不写文件、不执行命令；输出结论、证据路径和风险。".into(),
                tools: vec![
                    "read_file".into(),
                    "list_dir".into(),
                    "grep_files".into(),
                    "glob_files".into(),
                    "repo_map".into(),
                    "run_command".into(),
                    "load_skill".into(),
                ],
                steps: None, tool_budget: None, hidden: false, color: Some("#22D3EE".into()), source: AgentSource::Builtin,
            }),
            _ => None,
        }
    }
}
