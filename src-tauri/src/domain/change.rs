use serde::Serialize;
use std::collections::HashMap;

/// Line-level LCS diff count (added, removed) — bounded to ~4M cells; beyond
/// that every line counts as changed (same fallback as the old in-place code).
pub fn line_change_counts(before: &str, after: &str) -> (i64, i64) {
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    if a.len() * b.len() > 4_000_000 {
        return (b.len() as i64, a.len() as i64);
    }
    let mut dp = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut added, mut removed) = (0, 0, 0i64, 0i64);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] { i += 1; j += 1; }
        else if dp[i + 1][j] >= dp[i][j + 1] { removed += 1; i += 1; }
        else { added += 1; j += 1; }
    }
    (added + (b.len() - j) as i64, removed + (a.len() - i) as i64)
}

/// Merge per-write change records into one row per path: first `before`,
/// latest `after`, and net added/removed (git-stat semantics — the same
/// diff the review panel renders), instead of summed per-write churn.
pub fn merge_file_changes(changes: Vec<RunFileChange>) -> Vec<RunFileChange> {
    let mut order: Vec<String> = Vec::new();
    let mut merged: HashMap<String, RunFileChange> = HashMap::new();
    for change in changes {
        match merged.get_mut(&change.path) {
            Some(existing) => {
                existing.after = change.after;
                existing.created_at = existing.created_at.max(change.created_at);
            }
            None => {
                order.push(change.path.clone());
                merged.insert(change.path.clone(), change);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|path| merged.remove(&path))
        .map(|mut change| {
            let (added, removed) = line_change_counts(&change.before, &change.after);
            change.added = added;
            change.removed = removed;
            change
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFileChange {
    pub path: String,
    pub added: i64,
    pub removed: i64,
    pub before: String,
    pub after: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    pub reverted: Vec<String>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChangeSummary {
    pub files: i64,
    pub additions: i64,
    pub deletions: i64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertIssue {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertPreview {
    pub safe_files: Vec<String>,
    pub unsafe_files: Vec<RevertIssue>,
    pub ignored_files: Vec<RevertIssue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, before: &str, after: &str, at: i64) -> RunFileChange {
        let (added, removed) = line_change_counts(before, after);
        RunFileChange { path: path.into(), added, removed, before: before.into(), after: after.into(), created_at: at }
    }

    #[test]
    fn merge_uses_first_before_and_last_after_with_net_counts() {
        // 同一文件两次写入：added/removed 必须是原始→最终的净差异，
        // 而不是每次写入 diff 的加总。
        let merged = merge_file_changes(vec![
            change("a.cs", "x\ny", "x\na\nb\nc\ny", 1),   // +3/−0
            change("a.cs", "x\na\nb\nc\ny", "x\na\nb\nc\ny\nz", 2), // +1/−0
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].before, "x\ny");
        assert_eq!(merged[0].after, "x\na\nb\nc\ny\nz");
        assert_eq!((merged[0].added, merged[0].removed), (4, 0));
        assert_eq!(merged[0].created_at, 2);
    }

    #[test]
    fn merge_preserves_path_order_and_single_records() {
        let merged = merge_file_changes(vec![
            change("b.cs", "1", "1\n2", 1),
            change("a.cs", "p\nq", "q", 3),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].path, "b.cs");
        assert_eq!((merged[0].added, merged[0].removed), (1, 0));
        assert_eq!((merged[1].added, merged[1].removed), (0, 1));
    }
}
