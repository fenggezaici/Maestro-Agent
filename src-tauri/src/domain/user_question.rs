use serde::{Deserialize, Serialize};

use crate::domain::AppResult;

pub const MAX_QUESTIONS: usize = 4;
pub const MAX_OPTIONS: usize = 4;
pub const MAX_PAYLOAD_BYTES: usize = 8192;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    #[serde(default)]
    pub header: Option<String>,
    pub question: String,
    #[serde(default)]
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub custom: Option<String>,
}

/// Decode and bound the `ask_user` tool arguments.
pub fn parse_questions(arguments: &serde_json::Value) -> AppResult<Vec<Question>> {
    let raw = arguments
        .get("questions")
        .cloned()
        .ok_or_else(|| "ask_user 需要 questions 参数".to_string())?;
    let questions: Vec<Question> =
        serde_json::from_value(raw).map_err(|error| format!("ask_user 的 questions 格式无效：{error}"))?;
    if questions.is_empty() || questions.len() > MAX_QUESTIONS {
        return Err(format!("ask_user 需要 1–{MAX_QUESTIONS} 个问题"));
    }
    let mut seen = std::collections::HashSet::new();
    for question in &questions {
        if question.id.trim().is_empty() || question.id.len() > 64 {
            return Err("问题 id 必须为 1–64 字符".into());
        }
        if !seen.insert(question.id.clone()) {
            return Err(format!("问题 id 重复：{}", question.id));
        }
        if question.question.trim().is_empty() || question.question.chars().count() > 500 {
            return Err("问题文本必须为 1–500 字符".into());
        }
        if question.options.len() < 2 || question.options.len() > MAX_OPTIONS {
            return Err(format!("每个问题需要 2–{MAX_OPTIONS} 个选项"));
        }
        let mut labels = std::collections::HashSet::new();
        for option in &question.options {
            let label = option.label.trim();
            if label.is_empty() || label.chars().count() > 100 {
                return Err("选项文本必须为 1–100 字符".into());
            }
            if !labels.insert(label.to_string()) {
                return Err(format!("选项重复：{label}"));
            }
        }
    }
    if serde_json::to_string(&questions).map_err(|error| error.to_string())?.len() > MAX_PAYLOAD_BYTES {
        return Err("ask_user 的问题负载超过 8KB".into());
    }
    Ok(questions)
}

/// Validate submitted answers against the questions; missing questions count as
/// skipped and are allowed (the UI exposes an explicit skip action).
pub fn validate_answers(questions: &[Question], answers: &[QuestionAnswer]) -> AppResult<()> {
    for answer in answers {
        let Some(question) = questions.iter().find(|item| item.id == answer.question_id) else {
            return Err(format!("未知问题 id：{}", answer.question_id));
        };
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!("问题 {} 只能选择一个选项", question.id));
        }
        for selected in &answer.selected {
            if !question.options.iter().any(|option| option.label == *selected) {
                return Err(format!("问题 {} 的选项无效：{selected}", question.id));
            }
        }
        if answer.custom.as_deref().is_some_and(|value| value.chars().count() > 2000) {
            return Err("自定义回答超过 2000 字符".into());
        }
    }
    Ok(())
}

/// Tool result shown to the model: one line per question plus any custom text.
pub fn format_answers(questions: &[Question], answers: &[QuestionAnswer]) -> String {
    let answered: Vec<&QuestionAnswer> = questions
        .iter()
        .filter_map(|question| answers.iter().find(|answer| answer.question_id == question.id))
        .filter(|answer| !answer.selected.is_empty() || answer.custom.as_deref().is_some_and(|value| !value.trim().is_empty()))
        .collect();
    if answered.is_empty() {
        return "用户跳过了提问（未提供答案）".into();
    }
    let mut lines = vec!["【用户回答】".to_string()];
    for question in questions {
        let Some(answer) = answers.iter().find(|answer| answer.question_id == question.id) else {
            continue;
        };
        let mut parts = answer.selected.clone();
        if let Some(custom) = answer.custom.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            parts.push(format!("其他：{custom}"));
        }
        if parts.is_empty() {
            continue;
        }
        let label = question.header.as_deref().filter(|value| !value.trim().is_empty()).unwrap_or(&question.question);
        lines.push(format!("- {label}: {}", parts.join("、")));
    }
    if lines.len() == 1 {
        return "用户跳过了提问（未提供答案）".into();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> serde_json::Value {
        json!({
            "questions": [
                { "id": "scope", "header": "范围", "question": "分析哪些目录？", "options": [
                    { "label": "src" }, { "label": "tests", "description": "测试目录" }
                ]},
                { "id": "mode", "question": "输出风格？", "multiSelect": true, "options": [
                    { "label": "简洁" }, { "label": "详细" }
                ]}
            ]
        })
    }

    #[test]
    fn parses_and_bounds_questions() {
        let questions = parse_questions(&payload()).unwrap();
        assert_eq!(questions.len(), 2);
        assert!(questions[1].multi_select);
        assert!(parse_questions(&json!({ "questions": [] })).is_err());
        assert!(parse_questions(&json!({ "questions": [{
            "id": "x", "question": "?", "options": [{ "label": "a" }]
        }] })).is_err());
        let mut too_long = payload();
        too_long["questions"][0]["question"] = json!("x".repeat(501));
        assert!(parse_questions(&too_long).is_err());
    }

    #[test]
    fn validates_answers_and_formats_them() {
        let questions = parse_questions(&payload()).unwrap();
        let answers = vec![
            QuestionAnswer { question_id: "scope".into(), selected: vec!["src".into()], custom: None },
            QuestionAnswer { question_id: "mode".into(), selected: vec!["简洁".into(), "详细".into()], custom: Some("加表格".into()) },
        ];
        validate_answers(&questions, &answers).unwrap();
        let text = format_answers(&questions, &answers);
        assert!(text.contains("范围: src"));
        assert!(text.contains("简洁、详细、其他：加表格"));
        assert!(format_answers(&questions, &[]).contains("跳过"));
        assert!(validate_answers(&questions, &[QuestionAnswer { question_id: "scope".into(), selected: vec!["nope".into()], custom: None }]).is_err());
    }
}
