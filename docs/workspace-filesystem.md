# Phase 2: readonly local workspace

## Run the real Explorer

Close the previous desktop window and stop its terminal with Ctrl+C. Then run:

```powershell
Set-Location E:\Maestro-Agent
pnpm.cmd tauri dev
```

Open the right-side **Explorer** tab. Debug builds default to the repository containing `src-tauri`, so this checkout reads `E:\Maestro-Agent` without a hardcoded production path.
Use the path field and **Open workspace** button to choose another absolute local project directory. Refresh reloads the selected tree.
Click a file to fetch its current UTF-8 contents for readonly Monaco preview. Empty directories remain visible.
Settings is fixed at the sidebar bottom; model, Plan/Execute, context choice and Tokens are inline in the composer.

`pnpm.cmd dev:ui` is still an explicit browser Mock preview and cannot access Rust/local files. Packaged release builds require selecting a workspace rather than guessing a working directory.

## IPC contracts

- `get_workspace_info()` returns `{ name, path }`; `path` may be null before selection.
- `select_workspace({ workspacePath })` explicitly registers a canonical root in Rust.
- `read_directory_tree({ workspacePath })` returns `FileNode[]` with `{ name, path, is_dir, children }`.
- `read_workspace_text({ workspacePath, path })` returns a bounded UTF-8 text string.

Node paths are workspace-relative with `/` separators. Tree metadata does not eagerly load file contents. Requests for a root other than the registered selection are rejected.
Commands are granted only to the local main window through `allow-workspace-read`; no generic filesystem-write or shell permission is added.
Workspace browsing is not an Agent tool and does not automatically add files to LLM context. Browsing another root does not migrate or relabel persisted chat sessions as belonging to it.

## Safety and responsiveness

- Canonicalization, directory enumeration, metadata checks, file opening/reading and sorting run inside `spawn_blocking`, separate from the SQLite OS thread and Agent mutation gate.
- One workspace operation runs at a time. Root switching and reads cannot overlap in the Rust service.
- Prunes `.git`, `node_modules`, `target`, `dist`, `.idea`, `.next`, `.cache`, `.pnpm`, case-insensitively at every level.
- Skips symlinks and all Windows reparse points, including junctions; rejects linked workspace ancestors and direct link reads.
- Rejects traversal, absolute text paths, alternate data streams, unsupported device/network roots and non-regular files.
- Scan limits: 24 levels, 10,000 examined entries, 2 MiB tree JSON and a cooperative 15-second deadline. Exhaustion is an error, not a silent partial tree.
- File limit: 1 MiB, enforced on the actual bounded read. Binary/NUL and invalid UTF-8 contents show an unsupported-preview error.
- Frontend independently versions tree and file requests. Old successes/errors cannot overwrite a new workspace or selection; failed live reads never silently fall back to samples.

These metadata/canonicalization checks do not provide handle-relative atomic no-follow traversal against a concurrently hostile filesystem. Residual TOCTOU races require stronger OS-handle-based traversal in a later hardening phase. An OS-blocked filesystem call cannot be forcibly terminated by dropping its async caller.

## Verified

Rust tests exercise real files, empty directories, pruning, text/binary/size limits, unauthorized roots, Windows junction rejection and actual Maestro repository enumeration.
Frontend tests cover exact IPC arguments, lazy activation, readonly disk source, stale file responses/errors and live/mock separation.
Browser captures validate the footer staying fixed during project-tree scrolling, inline composer alignment and unchanged resizable/Review behavior; browser samples are not evidence of native filesystem IPC execution.
