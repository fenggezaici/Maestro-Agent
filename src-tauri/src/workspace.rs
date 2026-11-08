//! Read-only, explicitly selected workspace browsing. Never run std::fs on async workers.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{State, WebviewWindow};
use tokio::sync::Semaphore;

const MAX_ENTRIES: usize = 10000;
const MAX_DEPTH: usize = 24;
const MAX_TREE_BYTES: usize = 2097152;
const MAX_FILE_BYTES: u64 = 1048576;
const IGNORED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".idea",
    ".next",
    ".cache",
    ".pnpm",
];

#[derive(Debug, Clone, Serialize)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub path: Option<String>,
}

#[derive(Default)]
struct Budget {
    examined: usize,
    path_bytes: usize,
}

pub struct WorkspaceService {
    root: Arc<Mutex<Option<PathBuf>>>,
    gate: Arc<Semaphore>,
}

impl Default for WorkspaceService {
    fn default() -> Self {
        Self {
            root: Arc::new(Mutex::new(None)),
            gate: Arc::new(Semaphore::new(1)),
        }
    }
}

impl WorkspaceService {
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Option<PathBuf>) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let permit = self
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "Workspace service is closed".to_string())?;
        let state = self.root.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            let mut root = state
                .lock()
                .map_err(|_| "Workspace state unavailable".to_string())?;
            operation(&mut root)
        })
        .await
        .map_err(|_| "Workspace task failed".to_string())?
    }

    pub async fn info(&self) -> Result<WorkspaceInfo, String> {
        self.run(|root| {
            if root.is_none() && cfg!(debug_assertions) {
                let project = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .ok_or("Development workspace unavailable")?;
                *root = Some(select_root(project.to_string_lossy().as_ref())?);
            }
            Ok(info(root.as_deref()))
        })
        .await
    }

    pub async fn select(&self, path: String) -> Result<WorkspaceInfo, String> {
        self.run(move |root| {
            *root = Some(select_root(&path)?);
            Ok(info(root.as_deref()))
        })
        .await
    }

    pub async fn tree(&self, path: String) -> Result<Vec<FileNode>, String> {
        self.run(move |root| {
            let approved = authorized(root.as_deref(), &path)?;
            let tree = scan(
                &approved,
                &approved,
                0,
                &mut Budget::default(),
                Instant::now(),
            )?;
            if serde_json::to_vec(&tree)
                .map_err(|_| "Cannot encode workspace tree")?
                .len()
                > MAX_TREE_BYTES
            {
                return Err("Workspace tree exceeds 2 MiB limit".into());
            }
            Ok(tree)
        })
        .await
    }

    pub async fn text(&self, workspace: String, relative: String) -> Result<String, String> {
        self.run(move |root| {
            let approved = authorized(root.as_deref(), &workspace)?;
            read_text(&approved, &relative)
        })
        .await
    }
}

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Only the main window may browse a workspace".into())
    }
}

#[tauri::command]
pub async fn get_workspace_info(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
) -> Result<WorkspaceInfo, String> {
    require_main(&window)?;
    service.info().await
}

#[tauri::command]
pub async fn select_workspace(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
) -> Result<WorkspaceInfo, String> {
    require_main(&window)?;
    service.select(workspace_path).await
}

#[tauri::command]
pub async fn read_directory_tree(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
) -> Result<Vec<FileNode>, String> {
    require_main(&window)?;
    service.tree(workspace_path).await
}

#[tauri::command]
pub async fn read_workspace_text(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
    path: String,
) -> Result<String, String> {
    require_main(&window)?;
    service.text(workspace_path, path).await
}

fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    value.strip_prefix("\\\\?\\").unwrap_or(&value).to_string()
}

fn info(root: Option<&Path>) -> WorkspaceInfo {
    WorkspaceInfo {
        name: root
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Choose workspace".into()),
        path: root.map(display_path),
    }
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn ignored(name: &str) -> bool {
    IGNORED.iter().any(|entry| entry.eq_ignore_ascii_case(name))
}

fn select_root(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute()
        || value.trim().starts_with("\\\\")
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(
            "Choose an absolute local project directory (no network/device paths or traversal)"
                .into(),
        );
    }
    let mut ancestor = PathBuf::new();
    for component in path.components() {
        ancestor.push(component.as_os_str());
        if ancestor.is_absolute() {
            let metadata = fs::symlink_metadata(&ancestor)
                .map_err(|_| "Workspace directory does not exist or is inaccessible")?;
            if linked(&metadata) {
                return Err("Workspace ancestors cannot be links or reparse points".into());
            }
        }
    }
    let canonical = fs::canonicalize(path).map_err(|_| "Cannot resolve workspace directory")?;
    if !canonical.is_dir() || canonical.parent().is_none() {
        return Err("Choose a project folder, not a filesystem root".into());
    }
    Ok(canonical)
}

fn authorized(root: Option<&Path>, input: &str) -> Result<PathBuf, String> {
    let approved = root.ok_or("Choose a workspace before reading files")?;
    let candidate = select_root(input)?;
    if candidate != approved {
        return Err("This workspace has not been selected in the UI".into());
    }
    Ok(candidate)
}

fn scan(
    root: &Path,
    directory: &Path,
    depth: usize,
    budget: &mut Budget,
    started: Instant,
) -> Result<Vec<FileNode>, String> {
    if depth > MAX_DEPTH || started.elapsed() > Duration::from_secs(15) {
        return Err("Workspace scan exceeded depth/time limit".into());
    }
    let entries = fs::read_dir(directory).map_err(|_| "Cannot read a workspace directory")?;
    let mut nodes = Vec::new();
    for entry in entries {
        if started.elapsed() > Duration::from_secs(15) {
            return Err("Workspace scan exceeded time limit".into());
        }
        budget.examined += 1;
        if budget.examined > MAX_ENTRIES {
            return Err("Workspace has more than 10,000 entries; choose a smaller folder".into());
        }
        let entry = entry.map_err(|_| "Cannot read workspace entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| "Workspace entry became inaccessible")?;
        if linked(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
            continue;
        }
        let path = fs::canonicalize(entry.path()).map_err(|_| "Cannot resolve workspace entry")?;
        if !path.starts_with(root) {
            return Err("Workspace entry escapes selected root".into());
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Invalid workspace entry")?
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        budget.path_bytes += relative.len() + name.len();
        if budget.path_bytes > MAX_TREE_BYTES / 2 {
            return Err("Workspace tree exceeds size limit".into());
        }
        let children = if metadata.is_dir() {
            scan(root, &path, depth + 1, budget, started)?
        } else {
            Vec::new()
        };
        nodes.push(FileNode {
            name,
            path: relative,
            is_dir: metadata.is_dir(),
            children,
        });
    }
    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(nodes)
}

fn read_text(root: &Path, relative: &str) -> Result<String, String> {
    if relative.is_empty()
        || relative.contains(['\\', ':'])
        || relative
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | "..") || ignored(part))
    {
        return Err("Invalid or excluded workspace-relative file path".into());
    }
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "File is inaccessible or no longer exists")?;
        if linked(&metadata) {
            return Err("Link and reparse-point file reads are not allowed".into());
        }
    }
    let canonical = fs::canonicalize(&path).map_err(|_| "Cannot resolve file")?;
    if !canonical.starts_with(root) {
        return Err("File is outside selected workspace".into());
    }
    let before_open =
        fs::symlink_metadata(&canonical).map_err(|_| "Cannot inspect workspace file")?;
    if linked(&before_open) || !before_open.is_file() {
        return Err("Only regular, non-link workspace files can be previewed".into());
    }
    let file = fs::File::open(&canonical).map_err(|_| "Cannot open workspace file")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect workspace file")?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("Preview supports regular text files up to 1 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read workspace file")?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return Err("File is too large or binary; text preview unavailable".into());
    }
    String::from_utf8(bytes).map_err(|_| "File is not valid UTF-8; text preview unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_actual_files_empty_folders_and_prunes_ignored_directories() {
        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        fs::create_dir(temporary.path().join("empty")).unwrap();
        fs::write(temporary.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        for name in [".git", "node_modules", "target"] {
            fs::create_dir(temporary.path().join(name)).unwrap();
            fs::write(temporary.path().join(name).join("secret.txt"), "ignored").unwrap();
        }
        let root = select_root(temporary.path().to_str().unwrap()).unwrap();
        let tree = scan(&root, &root, 0, &mut Budget::default(), Instant::now()).unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "empty");
        assert!(tree[0].is_dir && tree[0].children.is_empty());
        assert_eq!(tree[1].children[0].path, "src/main.rs");
        assert_eq!(read_text(&root, "src/main.rs").unwrap(), "fn main() {}\n");
        for path in [
            "../outside",
            "/absolute",
            "target/secret.txt",
            "src/main.rs:stream",
            "src\\main.rs",
        ] {
            assert!(read_text(&root, path).is_err());
        }
        assert_eq!(serde_json::to_value(&tree[0]).unwrap()["is_dir"], true);
    }

    #[test]
    fn rejects_unapproved_roots_binary_and_large_files() {
        let temporary = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = select_root(temporary.path().to_str().unwrap()).unwrap();
        assert!(authorized(Some(&root), other.path().to_str().unwrap()).is_err());
        fs::write(root.join("binary"), [1, 0, 3]).unwrap();
        fs::write(root.join("large"), vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(read_text(&root, "binary").is_err());
        assert!(read_text(&root, "large").is_err());
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn default_service_reads_this_actual_repository_without_build_directories() {
        let service = WorkspaceService::default();
        let workspace = service.info().await.unwrap();
        let path = workspace.path.unwrap();
        let nodes = service.tree(path.clone()).await.unwrap();
        assert!(nodes.iter().any(|node| node.name == "Cargo.toml"));
        assert!(!nodes.iter().any(|node| ignored(&node.name)));
        assert!(service
            .text(path, "Cargo.toml".into())
            .await
            .unwrap()
            .contains("[workspace]"));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_directory_junctions() {
        let temporary = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        let junction = temporary.path().join("junction");
        let result = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(result.status.success(), "Cannot create test junction");
        let root = select_root(temporary.path().to_str().unwrap()).unwrap();
        assert!(linked(&fs::symlink_metadata(&junction).unwrap()));
        assert!(
            scan(&root, &root, 0, &mut Budget::default(), Instant::now())
                .unwrap()
                .is_empty()
        );
        assert!(read_text(&root, "junction/outside.txt").is_err());
        fs::remove_dir(junction).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn skips_links_and_rejects_link_reads() {
        let temporary = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), temporary.path().join("linked")).unwrap();
        let root = select_root(temporary.path().to_str().unwrap()).unwrap();
        assert!(
            scan(&root, &root, 0, &mut Budget::default(), Instant::now())
                .unwrap()
                .is_empty()
        );
        assert!(read_text(&root, "linked/file.txt").is_err());
    }
}
