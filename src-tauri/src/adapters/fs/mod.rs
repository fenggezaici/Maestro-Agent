//! Filesystem adapter: read-only, explicitly selected workspace browsing.
pub mod scoped;
pub mod workspace;

pub use scoped::{apply_patch, list_dir, read_file, scoped_path, scoped_write_path, write_file};
pub use workspace::{FileNode, WorkspaceInfo, WorkspaceService};
