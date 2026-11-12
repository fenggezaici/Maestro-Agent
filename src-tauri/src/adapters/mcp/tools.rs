//! Tool registry helpers: `mcp__` namespacing, `tools/list` parsing, and
//! flattening `tools/call` results into bounded display text.
use serde_json::Value;

use crate::domain::McpToolView;

pub const MAX_TOOL_RESULT_BYTES: usize = 65536;
pub const MAX_TOOLS_PER_SERVER: usize = 256;

/// `readOnlyHint` is self-declared by the child process. Only an explicitly
/// trusted server may use it to become read-only; untrusted servers are always
/// treated as side-effecting (fail-closed).
pub fn effective_read_only(trust: bool, read_only: bool) -> bool {
    trust && read_only
}

/// Split `mcp__<server>__<tool>` into (server, tool); returns None otherwise.
pub fn split_mcp_tool(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() || tool.contains("__") {
        return None;
    }
    Some((server, tool))
}

pub fn parse_tools(listed: &Value) -> Vec<McpToolView> {
    listed
        .get("tools")
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.to_string();
                    if name.len() > 128 {
                        return None;
                    }
                    Some(McpToolView {
                        name,
                        description: tool
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default()
                            .chars()
                            .take(512)
                            .collect(),
                        input_schema: tool
                            .get("inputSchema")
                            .filter(|schema| schema.is_object())
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({ "type": "object" })),
                        read_only: tool
                            .pointer("/annotations/readOnlyHint")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    })
                })
                .take(MAX_TOOLS_PER_SERVER)
                .collect()
        })
        .unwrap_or_default()
}

/// Interpret one `tools/call` result for the agent loop: a result flagged
/// `isError` is a tool-reported failure and must surface as `Err` — the model
/// never gets to treat a failed call as a success.
pub fn call_result_text(result: &Value) -> Result<String, String> {
    let text = tool_result_text(result);
    if result
        .get("isError")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false)
    {
        Err(text)
    } else {
        Ok(text)
    }
}

/// Flatten an MCP tools/call result into bounded display text.
pub fn tool_result_text(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(|content| content.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|joined| !joined.is_empty())
        .unwrap_or_else(|| serde_json::to_string(result).unwrap_or_default());
    text.chars().take(MAX_TOOL_RESULT_BYTES).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_tool_names_split_once() {
        assert_eq!(split_mcp_tool("mcp__fs__read_file"), Some(("fs", "read_file")));
        assert_eq!(split_mcp_tool("mcp__a-b_c__t"), Some(("a-b_c", "t")));
        assert_eq!(split_mcp_tool("load_skill"), None);
        assert_eq!(split_mcp_tool("mcp__fs"), None);
        assert_eq!(split_mcp_tool("mcp____x"), None);
        assert_eq!(split_mcp_tool("mcp__fs__a__b"), None);
    }

    #[test]
    fn iserror_results_surface_as_failures() {
        let ok = json!({ "content": [{"type":"text","text":"done"}] });
        assert_eq!(call_result_text(&ok).unwrap(), "done");
        let failed = json!({ "isError": true, "content": [{"type":"text","text":"boom"}] });
        assert_eq!(call_result_text(&failed).unwrap_err(), "boom");
        let flagged_null = json!({ "isError": true });
        assert!(call_result_text(&flagged_null).is_err());
    }

    #[test]
    fn tool_result_text_flattens_content_blocks() {
        let result = json!({ "content": [{"type":"text","text":"hello"},{"type":"text","text":"world"}] });
        assert_eq!(tool_result_text(&result), "hello\nworld");
        let fallback = json!({ "structured": true });
        assert!(tool_result_text(&fallback).contains("structured"));
    }

    #[test]
    fn parses_tools_with_read_only_annotations() {        let listed = json!({ "tools": [
            {"name":"read_file","description":"reads","annotations":{"readOnlyHint":true}},
            {"name":"write_file","description":"writes"},
        ]});
        let tools = parse_tools(&listed);
        assert_eq!(tools.len(), 2);
        assert!(tools[0].read_only && !tools[1].read_only);
        assert!(parse_tools(&json!({})).is_empty());
    }

    #[test]
    fn read_only_hint_requires_explicit_trust() {
        assert!(!effective_read_only(false, true));
        assert!(!effective_read_only(false, false));
        assert!(!effective_read_only(true, false));
        assert!(effective_read_only(true, true));
    }
}
