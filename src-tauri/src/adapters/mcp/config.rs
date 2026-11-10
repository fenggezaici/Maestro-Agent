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
    valid_server_name(name)?;
    if config.command.trim().is_empty() || config.command.len() > 1024 {
        return Err("MCP command 为空或过长".into());
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
}
