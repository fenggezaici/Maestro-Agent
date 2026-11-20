//! Filesystem adapter: read-only, explicitly selected workspace browsing.
pub mod scoped;
pub mod workspace;

pub use scoped::{
    apply_patch, apply_patch_multi, glob_files, grep_files, is_sensitive_path, list_dir, read_file,
    read_file_full, read_file_lines, remove_file, scoped_path, scoped_write_path, write_file,
    WriteReport,
};
pub use workspace::{FileNode, WorkspaceInfo, WorkspaceService};
