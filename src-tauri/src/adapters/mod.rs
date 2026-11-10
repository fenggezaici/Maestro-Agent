//! Adapter layer: everything that touches the outside world (SQLite, the
//! filesystem, MCP child processes, LLM HTTP transports).
pub mod db;
pub mod fs;
pub mod mcp;
pub mod skills;
