//! End-to-end stdio MCP test against a real Node child process. The fake server
//! implements only the four methods the client uses and declares a readOnlyHint
//! so trust gating can be asserted too.
use std::fs;

use maestro_lib::adapters::mcp::McpManager;
use maestro_lib::domain::McpServerConfig;
use serde_json::json;

const FAKE_SERVER: &str = r#"
const readline = require('node:readline');
const rl = readline.createInterface({ input: process.stdin });
const send = (id, result, error) => {
  const payload = { jsonrpc: '2.0', id };
  if (error) payload.error = { message: error }; else payload.result = result;
  process.stdout.write(JSON.stringify(payload) + '\n');
};
rl.on('line', (line) => {
  if (!line.trim()) return;
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  if (msg.id === undefined) return;
  if (msg.method === 'initialize') {
    send(msg.id, { protocolVersion: '2025-03-26', capabilities: {}, serverInfo: { name: 'fake', version: '1' } });
  } else if (msg.method === 'tools/list') {
    send(msg.id, { tools: [{ name: 'echo', description: 'echo back', inputSchema: { type: 'object', properties: { text: { type: 'string' } } }, annotations: { readOnlyHint: true } }] });
  } else if (msg.method === 'tools/call') {
    send(msg.id, { content: [{ type: 'text', text: 'echo:' + (msg.params.arguments && msg.params.arguments.text ? msg.params.arguments.text : '') }] });
  } else {
    send(msg.id, {});
  }
});
"#;

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn fake_stdio_server_lists_calls_and_respects_trust() {
    if !node_available() {
        eprintln!("skip: node is not available on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-mcp.js");
    fs::write(&script, FAKE_SERVER).unwrap();
    let config_path = dir.path().join("mcp.json");

    let manager = McpManager::default();
    manager.set_config_path(config_path.clone());
    manager
        .upsert_server(
            "fake",
            McpServerConfig {
                command: "node".into(),
                args: vec![script.to_string_lossy().into_owned()],
                env: Default::default(),
                disabled: false,
                auto_start: false,
                trust: false,
            },
        )
        .unwrap();

    let view = manager.start("fake").await.unwrap();
    assert!(view.running);
    assert_eq!(view.tool_count, 1);

    // An untrusted server never becomes read-only from its own annotation.
    assert!(!manager.tool_is_read_only("mcp__fake__echo").await);
    manager.set_trust("fake", true).await.unwrap();
    assert!(manager.tool_is_read_only("mcp__fake__echo").await);

    let text = manager
        .call_tool("mcp__fake__echo", json!({ "text": "hi" }))
        .await
        .unwrap();
    assert_eq!(text, "echo:hi");

    let disabled = manager.disable("fake").await.unwrap();
    assert!(disabled.disabled);
    assert!(!disabled.running);
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["fake"]["disabled"], json!(true));
    assert!(manager.start("fake").await.is_err());

    // Startup restore ignores disabled entries.
    manager.start_enabled().await;
    assert!(!manager.list_servers().await.unwrap()[0].running);

    manager.shutdown().await;
}
