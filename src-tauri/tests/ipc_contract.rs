//! Guards the three-way IPC contract:
//! webview API (`src/api/*.ts` invoke names) ↔ command registry
//! (`generate_handler!` in `lib.rs`) ↔ capability ACL (`permissions/*.toml`).
//!
//! Each hop has already failed once in practice: a new command was registered
//! and called but not added to the ACL, so the packaged app answered
//! "… not allowed. Command not found." while every unit test stayed green.
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("src-tauri must live inside the repository")
        .to_path_buf()
}

/// Command names registered in `lib.rs`'s `generate_handler![ … ]` block.
fn registered_commands() -> Vec<String> {
    let lib = std::fs::read_to_string(manifest_dir().join("src/lib.rs")).expect("read src/lib.rs");
    let start = lib.find("generate_handler![").expect("generate_handler! block");
    let end = start + lib[start..].find(']').expect("handler list terminator");
    let commands = lib[start..end]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("commands::"))
        .filter_map(|line| line.trim_end_matches(',').rsplit("::").next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(
        !commands.is_empty(),
        "failed to parse generate_handler! entries — the guard would pass vacuously"
    );
    commands
}

/// Every command allowed by any permission file.
fn allowed_commands() -> Vec<String> {
    let dir = manifest_dir().join("permissions");
    let mut allowed = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("permissions directory") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read permission file");
        for line in text.lines() {
            let Some(rest) = line.split("commands.allow").nth(1) else {
                continue;
            };
            for value in rest.split('"').skip(1).step_by(2) {
                if !value.is_empty() {
                    allowed.push(value.to_string());
                }
            }
        }
    }
    assert!(
        !allowed.is_empty(),
        "failed to parse commands.allow — the guard would pass vacuously"
    );
    allowed
}

/// Command names the webview actually calls: the first string literal after
/// each `invoke` (generic arguments are skipped by searching for the quote).
fn invoked_commands() -> Vec<String> {
    let dir = repo_root().join("src/api");
    let mut names = Vec::new();
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).expect("src/api directory") {
        let path = entry.unwrap().path();
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default();
        if path.extension().and_then(|value| value.to_str()) != Some("ts") || name.ends_with(".test.ts") {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&path).expect("read api module");
        names.extend(command_names_after(&text, "invoke"));
    }
    assert!(files > 0, "no API modules found under src/api");
    names.sort();
    names.dedup();
    assert!(
        !names.is_empty(),
        "failed to parse invoke calls — the guard would pass vacuously"
    );
    names
}

fn command_names_after(text: &str, needle: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut index = 0;
    while let Some(found) = text[index..].find(needle) {
        let start = index + found;
        // Only a bare `invoke(...)` call counts: `mocks.invoke.mockReset()` and
        // prose mentioning invoke must not produce a command name.
        let preceded_by_member = text[..start].chars().next_back().is_some_and(|value| {
            value.is_alphanumeric() || matches!(value, '.' | '_' | '$')
        });
        let mut cursor = start + needle.len();
        if preceded_by_member {
            index = cursor;
            continue;
        }
        // Skip an optional generic argument list (`invoke<void>(`).
        let rest = text[cursor..].trim_start();
        cursor = text.len() - rest.len();
        if rest.starts_with('<') {
            let mut depth = 0usize;
            let mut closed = None;
            for (offset, value) in rest.char_indices() {
                match value {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            closed = Some(offset);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(offset) = closed else {
                index = cursor;
                continue;
            };
            cursor += offset + 1;
        }
        let rest = text[cursor..].trim_start();
        cursor = text.len() - rest.len();
        if !rest.starts_with('(') {
            index = cursor;
            continue;
        }
        let rest = rest[1..].trim_start();
        cursor += 1 + (text[cursor + 1..].len() - text[cursor + 1..].trim_start().len());
        let Some(quote) = rest.chars().next().filter(|value| matches!(value, '\'' | '"')) else {
            index = cursor;
            continue;
        };
        let after = &rest[1..];
        if let Some(end) = after.find(quote) {
            let name = &after[..end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || value == '_')
            {
                names.push(name.to_string());
            }
            index = cursor + 1 + end + 1;
            continue;
        }
        index = cursor;
    }
    names
}

#[test]
fn every_invoked_command_is_registered() {
    let registered = registered_commands();
    let missing: Vec<_> = invoked_commands()
        .into_iter()
        .filter(|name| !registered.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "the webview calls commands that are not registered in generate_handler!: {missing:?}"
    );
}

#[test]
fn every_invoked_command_is_allowed_by_a_capability() {
    let allowed = allowed_commands();
    let missing: Vec<_> = invoked_commands()
        .into_iter()
        .filter(|name| !allowed.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "the webview calls commands the ACL does not allow (runtime says 'not allowed. Command not found.'): {missing:?}"
    );
}

#[test]
fn every_registered_command_is_reachable_by_the_webview() {
    // The reverse direction: a command nobody calls is either dead code or a
    // typo on the TypeScript side.
    let invoked = invoked_commands();
    let unused: Vec<_> = registered_commands()
        .into_iter()
        .filter(|name| !invoked.contains(name))
        .collect();
    assert!(
        unused.is_empty(),
        "registered commands that no src/api module invokes: {unused:?}"
    );
}
