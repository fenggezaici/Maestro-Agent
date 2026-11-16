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
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "default_source")]
    pub source: AgentSource,
}

fn default_mode() -> String { "subagent".into() }

impl AgentDefinition {
    pub fn builtin(name: &str) -> Option<Self> {
        match name {
            "explore" => Some(Self {
                name: "explore".into(),
                description: "只读代码调研：并行查找文件、证据路径与风险".into(),
                category: Some("研究".into()), mode: "subagent".into(), model: None,
                prompt: "你是只读代码调研子智能体。只使用提供的只读工具并行查找证据，不写文件、不执行命令；输出结论、证据路径和风险。".into(),
                tools: vec!["read_file".into(), "list_dir".into(), "load_skill".into()],
                steps: Some(8), hidden: false, color: Some("#8B5CF6".into()), source: AgentSource::Builtin,
            }),
            "general" => Some(Self {
                name: "general".into(),
                description: "只读分析总结：整理多来源信息并给出结论".into(),
                category: Some("研究".into()), mode: "subagent".into(), model: None,
                prompt: "你是只读分析子智能体。只使用提供的只读工具分析，不写文件、不执行命令；输出结论、证据路径和风险。".into(),
                tools: vec!["read_file".into(), "list_dir".into(), "load_skill".into()],
                steps: Some(8), hidden: false, color: Some("#22D3EE".into()), source: AgentSource::Builtin,
            }),
            _ => None,
        }
    }
}
