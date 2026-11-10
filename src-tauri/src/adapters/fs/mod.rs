//! Filesystem adapter: read-only, explicitly selected workspace browsing.
pub mod scoped;
pub mod workspace;

pub use scoped::{list_dir, read_file, scoped_path};
pub use workspace::{FileNode, WorkspaceInfo, WorkspaceService};
