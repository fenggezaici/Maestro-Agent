//! `mcp.json` persistence and validation for MCP server entries.
use std::{fs, path::PathBuf};

use crate::domain::{AppResult, McpConfigFile, McpServerConfig};

pub fn valid_server_name(name: &str) -> AppResult<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("MCP 服务名必须为 1-64 位字母、数字、下划线或连字符".into());
    }
    Ok(trimmed)
}

pub fn valid_server_config(name: &str, config: &McpServerConfig) -> AppResult<McpServerConfig> {
    use crate::domain::McpTransport;
    valid_server_name(name)?;
    match config.transport {
        McpTransport::Stdio => {
            if config.command.trim().is_empty() || config.command.len() > 1024 {
                return Err("MCP command 为空或过长".into());
            }
        }
        McpTransport::Http | McpTransport::Sse => {
            let url = config.url.as_deref().unwrap_or("").trim();
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err("远程 MCP 必须配置 http(s) URL".into());
            }
            if reqwest::Url::parse(url).is_err() {
                return Err("MCP URL 不是合法地址".into());
            }
        }
    }
    if config.args.len() > 64 || config.args.iter().any(|arg| arg.len() > 1024) {
        return Err("MCP args 超过数量/长度限制".into());
    }
    if config.env.len() > 64
        || config
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.len() > 128 || value.len() > 4096)
    {
        return Err("MCP env 超过数量/长度限制".into());
    }
    if config.headers.len() > 32
        || config.headers.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || value.len() > 4096
                || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return Err("MCP 请求头超过数量/格式限制".into());
    }
    if let Some(ms) = config.timeout_ms {
        if !(1_000..=600_000).contains(&ms) {
            return Err("MCP timeoutMs 需在 1–600 秒之间".into());
        }
    }
    Ok(config.clone())
}

/// Read `mcp.json`; a missing file yields an empty config, a corrupt file errors.
pub fn load_config(path: &PathBuf) -> AppResult<McpConfigFile> {
    if !path.is_file() {
        return Ok(McpConfigFile::default());
    }
    let content = fs::read_to_string(path).map_err(|_| "无法读取 mcp.json".to_string())?;
    serde_json::from_str(&content).map_err(|_| "mcp.json 格式无效".to_string())
}

pub fn save_config(path: &PathBuf, config: &McpConfigFile) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建配置目录".to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|_| "无法序列化配置".to_string())?;
    fs::write(path, content).map_err(|_| "无法写入 mcp.json".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert!(load_config(&path).unwrap().mcp_servers.is_empty());
        fs::write(&path, r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","server-fs"]}}}"#).unwrap();
        let file = load_config(&path).unwrap();
        assert_eq!(file.mcp_servers["fs"].args.len(), 2);
        fs::write(&path, "not json").unwrap();
        assert!(load_config(&path).is_err());
        assert!(valid_server_name("bad name!").is_err());
        assert!(valid_server_name("good-name_1").is_ok());
    }

    #[test]
    fn timeout_ms_roundtrips_camel_case_and_is_bounded() {
        let mut server: McpServerConfig = serde_json::from_str(
            r#"{"command":"npx","timeoutMs":45000}"#,
        )
        .unwrap();
        assert_eq!(server.timeout_ms, Some(45_000));
        assert!(serde_json::to_string(&server).unwrap().contains("timeoutMs"));
        assert!(valid_server_config("fs", &server).is_ok());
        server.timeout_ms = Some(500);
        assert!(valid_server_config("fs", &server).is_err());
        server.timeout_ms = Some(700_000);
        assert!(valid_server_config("fs", &server).is_err());
        server.timeout_ms = None;
        assert!(valid_server_config("fs", &server).is_ok());
    }
}
