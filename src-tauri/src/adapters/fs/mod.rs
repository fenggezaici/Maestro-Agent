//! Filesystem adapter: read-only, explicitly selected workspace browsing.
pub mod workspace;

pub use workspace::{FileNode, WorkspaceInfo, WorkspaceService};
