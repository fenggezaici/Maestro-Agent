use serde::{Deserialize, Serialize};

/// User decision for a suspended tool batch.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Deny,
    /// 批准当前调用，本批后续挂起调用自动放行（批内有效，不外泄）。
    AllowBatch,
    /// 拒绝当前调用，本批后续挂起调用自动按同一理由拒绝。
    DenyBatch,
    /// 批准当前调用，本会话后续所有 RequireApproval 调用免审
    /// （ZCode "full access" 对应物；纯内存、可撤销，不解除 Plan 的拒绝）。
    AllowSession,
}

impl ApprovalDecision {
    /// 该决策对当前挂起调用本身意味着什么。
    pub fn is_allow(self) -> bool {
        matches!(
            self,
            Self::AllowOnce | Self::AllowAlways | Self::AllowBatch | Self::AllowSession
        )
    }
}

/// How wide an "always allow" rule may be. `Exact` is the default: the rule
/// matches the full command (or the full argument identity) and nothing else.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    Prefix,
    #[default]
    Exact,
}

/// Longest rule pattern we are willing to store. Anything wider has no bounded
/// identity, so the call keeps asking instead of inventing a catch-all rule.
pub const MAX_RULE_PATTERN_CHARS: usize = 4096;

/// A suspended call from which an "always allow" rule can be derived.
#[derive(Debug, Clone)]
pub struct ApprovalCandidate {
    pub tool: String,
    pub target: String,
    pub command: Option<String>,
    /// False when the call has no bounded identity (oversized arguments): no
    /// durable rule may be derived from it, and no existing rule may match it.
    pub scopable: bool,
}

/// Session-scoped rule. `prefix` only applies to `run_command`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRule {
    pub tool: String,
    pub pattern: String,
    pub prefix: bool,
}

/// Canonical JSON for MCP arguments: object keys are sorted recursively so the
/// same call always yields the same rule pattern, while a different argument
/// set never reuses an earlier approval.
pub fn canonical_json(value: &serde_json::Value) -> String {
    fn write(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).unwrap_or_default());
                    out.push(':');
                    write(&map[*key], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

impl ApprovalRule {
    /// Derive the rule an "always allow" decision would store. `None` means the
    /// call has no bounded identity, so the decision must stay per-call.
    pub fn from_candidate(candidate: &ApprovalCandidate, scope: CommandScope) -> Option<Self> {
        if !candidate.scopable {
            return None;
        }
        if let Some(command) = candidate.command.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            let (pattern, prefix) = match scope {
                CommandScope::Prefix => (
                    command.split_whitespace().next().unwrap_or(command).to_string(),
                    true,
                ),
                CommandScope::Exact => (command.to_string(), false),
            };
            if pattern.chars().count() > MAX_RULE_PATTERN_CHARS {
                return None;
            }
            return Some(Self { tool: candidate.tool.clone(), pattern, prefix });
        }
        let pattern = candidate.target.trim();
        if pattern.is_empty() || pattern.chars().count() > MAX_RULE_PATTERN_CHARS {
            return None;
        }
        Some(Self { tool: candidate.tool.clone(), pattern: pattern.to_string(), prefix: false })
    }

    pub fn matches(&self, candidate: &ApprovalCandidate) -> bool {
        if !candidate.scopable || self.tool != candidate.tool {
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
        ApprovalCandidate {
            tool: "run_command".into(),
            target: value.into(),
            command: Some(value.into()),
            scopable: true,
        }
    }

    fn mcp(arguments: &str) -> ApprovalCandidate {
        let value: serde_json::Value = serde_json::from_str(arguments).unwrap();
        let target = canonical_json(&value);
        let scopable = target.chars().count() <= MAX_RULE_PATTERN_CHARS;
        ApprovalCandidate {
            tool: "mcp__fs__write".into(),
            target: if scopable { target } else { String::new() },
            command: None,
            scopable,
        }
    }

    #[test]
    fn exact_is_the_default_scope() {
        assert_eq!(CommandScope::default(), CommandScope::Exact);
    }

    #[test]
    fn command_prefix_matches_same_binary_but_exact_does_not() {
        let prefix =
            ApprovalRule::from_candidate(&command("npm run test"), CommandScope::Prefix).unwrap();
        assert_eq!(prefix.pattern, "npm");
        assert!(prefix.matches(&command("npm run test")));
        assert!(prefix.matches(&command("npm install")));
        assert!(!prefix.matches(&command("pnpm install")));

        let exact =
            ApprovalRule::from_candidate(&command("npm run test"), CommandScope::Exact).unwrap();
        assert!(exact.matches(&command("npm run test")));
        assert!(!exact.matches(&command("npm install")));
    }

    #[test]
    fn file_rules_match_the_same_target_only() {
        let candidate = ApprovalCandidate {
            tool: "write_file".into(),
            target: "src/a.ts".into(),
            command: None,
            scopable: true,
        };
        let rule = ApprovalRule::from_candidate(&candidate, CommandScope::Prefix).unwrap();
        assert!(rule.matches(&candidate));
        assert!(!rule.matches(&ApprovalCandidate { target: "src/b.ts".into(), ..candidate.clone() }));
    }

    #[test]
    fn mcp_rules_are_keyed_by_the_full_argument_set() {
        let rule = ApprovalRule::from_candidate(
            &mcp(r#"{"path":"a.txt","content":"one"}"#),
            CommandScope::Exact,
        )
        .unwrap();
        assert!(rule.matches(&mcp(r#"{"content":"one","path":"a.txt"}"#)));
        assert!(!rule.matches(&mcp(r#"{"path":"a.txt","content":"two"}"#)));
        assert!(!rule.matches(&mcp(r#"{"path":"b.txt","content":"one"}"#)));
    }

    #[test]
    fn calls_without_a_bounded_identity_never_get_a_rule() {
        let oversized = ApprovalCandidate {
            tool: "mcp__fs__write".into(),
            target: String::new(),
            command: None,
            scopable: false,
        };
        assert!(ApprovalRule::from_candidate(&oversized, CommandScope::Exact).is_none());
        let empty_target = ApprovalCandidate {
            tool: "mcp__fs__write".into(),
            target: String::new(),
            command: None,
            scopable: true,
        };
        assert!(ApprovalRule::from_candidate(&empty_target, CommandScope::Exact).is_none());
        // A wide rule can never match an unbounded call either.
        let rule = ApprovalRule::from_candidate(&command("npm run test"), CommandScope::Prefix).unwrap();
        assert!(!rule.matches(&oversized));
    }
}
