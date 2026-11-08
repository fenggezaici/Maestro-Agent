use crate::model::{PlanDraft, ProposedDiff};

pub struct ModelReply {
    pub text: String,
    pub proposed_tool: &'static str,
    pub plan: PlanDraft,
}

pub struct PlaceholderModel;

impl PlaceholderModel {
    pub fn reply(&self, prompt: &str, turn: usize) -> ModelReply {
        let goal: String = prompt.chars().take(80).collect();
        ModelReply {
            text: format!("[占位模型 · 未连接真实服务]\n\n这是本会话的第 {turn} 轮请求。\n\n我会围绕“{goal}”先明确目标、列出实现步骤，再提交一个可审查的修改提议。\n\n计划已拆分为三个步骤。下面的 Diff 是占位提议，不是对工作区的真实读取或修改。"),
            proposed_tool: "apply_patch",
            plan: proposal(prompt),
        }
    }
}

pub(crate) fn proposal(prompt: &str) -> PlanDraft {
    let goal: String = prompt.chars().take(120).collect();
    PlanDraft {
        title: "实现计划".into(),
        summary: format!("目标：{goal}\n占位模型生成的提议。副作用工具必须经过 Rust 拦截；审批仅记录此计划的决定，不会执行修改。"),
        steps: vec![
            "确认需求、现有行为和变更边界。".into(),
            "按审查通过的具体范围实现，并检查提议 Diff。".into(),
            "补充回归测试，验证结果后再进行下一轮。".into(),
        ],
        diffs: vec![ProposedDiff {
            path: "docs/agent-plan.md".into(),
            before: String::new(),
            after: format!("# 待审查的计划（占位提议）\n\n目标：{goal}\n\n- 明确变更范围\n- 实现并审查 Diff\n- 验证回归测试\n"),
        }],
    }
}

pub fn tokens(text: &str) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|characters| characters.iter().collect())
        .collect()
}
