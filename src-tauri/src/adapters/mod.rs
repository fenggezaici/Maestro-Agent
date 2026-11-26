//! Adapter layer: everything that touches the outside world (SQLite, the
//! filesystem, MCP child processes, LLM HTTP transports).
pub mod agents;
pub mod commands;
pub mod credentials;
pub mod db;
pub mod exec;
pub mod fs;
pub mod git;
pub mod llm;
pub mod mcp;
pub mod media;
pub mod plugins;
pub mod search;
pub mod skills;
pub mod web;
