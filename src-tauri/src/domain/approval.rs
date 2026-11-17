use serde::Deserialize;

/// User decision for a suspended tool batch.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    Prefix,
    Exact,
}

/// A suspended call from which an "always allow" rule can be derived.
#[derive(Debug, Clone)]
pub struct ApprovalCandidate {
    pub tool: String,
    pub target: String,
    pub command: Option<String>,
}

/// Session-scoped rule. `prefix` only applies to `run_command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRule {
    pub tool: String,
    pub pattern: String,
    pub prefix: bool,
}

impl ApprovalRule {
    pub fn from_candidate(candidate: &ApprovalCandidate, scope: CommandScope) -> Self {
        if let Some(command) = candidate.command.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            let (pattern, prefix) = match scope {
                CommandScope::Prefix => (
                    command.split_whitespace().next().unwrap_or(command).to_string(),
                    true,
                ),
                CommandScope::Exact => (command.to_string(), false),
            };
            return Self { tool: candidate.tool.clone(), pattern, prefix };
        }
        Self { tool: candidate.tool.clone(), pattern: candidate.target.clone(), prefix: false }
    }

    pub fn matches(&self, candidate: &ApprovalCandidate) -> bool {
        if self.tool != candidate.tool {
            return false;
        }
        match candidate.command.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            Some(command) if self.prefix => {
                command == self.pattern || command.starts_with(&format!("{} ", self.pattern))
            }
            Some(command) => command == self.pattern,
            None => candidate.target == self.pattern,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(value: &str) -> ApprovalCandidate {
        ApprovalCandidate { tool: "run_command".into(), target: value.into(), command: Some(value.into()) }
    }

    #[test]
    fn command_prefix_matches_same_binary_but_exact_does_not() {
        let prefix = ApprovalRule::from_candidate(&command("npm run test"), CommandScope::Prefix);
        assert_eq!(prefix.pattern, "npm");
        assert!(prefix.matches(&command("npm run test")));
        assert!(prefix.matches(&command("npm install")));
        assert!(!prefix.matches(&command("pnpm install")));

        let exact = ApprovalRule::from_candidate(&command("npm run test"), CommandScope::Exact);
        assert!(exact.matches(&command("npm run test")));
        assert!(!exact.matches(&command("npm install")));
    }

    #[test]
    fn file_rules_match_the_same_target_only() {
        let candidate = ApprovalCandidate { tool: "write_file".into(), target: "src/a.ts".into(), command: None };
        let rule = ApprovalRule::from_candidate(&candidate, CommandScope::Prefix);
        assert!(rule.matches(&candidate));
        assert!(!rule.matches(&ApprovalCandidate { tool: "write_file".into(), target: "src/b.ts".into(), command: None }));
    }
}
