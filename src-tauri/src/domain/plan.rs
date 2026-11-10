use serde::Serialize;

use crate::domain::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    Approved,
    Rejected,
    Invalidated,
}

impl PlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Invalidated => "invalidated",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedDiff {
    pub path: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanDraft {
    pub title: String,
    pub summary: String,
    pub steps: Vec<String>,
    pub diffs: Vec<ProposedDiff>,
}

impl PlanDraft {
    pub fn validate(&self) -> AppResult<()> {
        if self.title.trim().is_empty()
            || self.title.len() > 256
            || self.summary.len() > 16384
            || self.steps.is_empty()
            || self.steps.len() > 20
            || self.steps.iter().any(|step| step.len() > 4096)
            || self.diffs.len() > 20
            || self
                .diffs
                .iter()
                .any(|diff| diff.path.len() > 512 || diff.before.len() + diff.after.len() > 65536)
        {
            return Err("Invalid or oversized plan proposal".into());
        }
        for diff in &self.diffs {
            if diff.path.is_empty()
                || diff.path.starts_with('/')
                || diff.path.contains(['\\', ':'])
                || diff
                    .path
                    .split('/')
                    .any(|part| part == ".." || part.is_empty())
            {
                return Err("Proposed diff paths must be relative and scoped".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCard {
    pub id: i64,
    pub session_id: i64,
    pub run_id: i64,
    pub status: PlanStatus,
    pub draft: PlanDraft,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_contract_and_paths_are_scoped() {
        let mut draft = PlanDraft {
            title: "proposal".into(),
            summary: "not applied".into(),
            steps: vec!["review".into()],
            diffs: vec![ProposedDiff {
                path: "docs/plan.md".into(),
                before: String::new(),
                after: "proposal".into(),
            }],
        };
        draft.validate().unwrap();
        for path in ["../outside", "/absolute", "C:\\file", "docs/../file"] {
            draft.diffs[0].path = path.into();
            assert!(draft.validate().is_err());
        }
    }
}
