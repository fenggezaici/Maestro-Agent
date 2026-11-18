//! Guards the Tauri app-command ACL.
//!
//! Once `permissions/*.toml` exists, Tauri treats the app as having an ACL
//! manifest and rejects every command that is registered in `generate_handler!`
//! but not listed in a capability's `commands.allow`. Nothing else catches this:
//! frontend tests mock `invoke` and Rust tests call the kernel directly, so both
//! stay green while the packaged app answers "… not allowed. Command not found."
use std::path::Path;

fn manifest_dir() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// Command names registered in `lib.rs`'s `generate_handler![ … ]` block.
fn registered_commands() -> Vec<String> {
    let lib = std::fs::read_to_string(Path::new(manifest_dir()).join("src/lib.rs"))
        .expect("read src/lib.rs");
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
    let dir = Path::new(manifest_dir()).join("permissions");
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

#[test]
fn every_registered_command_is_allowed_by_a_capability() {
    let allowed = allowed_commands();
    let missing: Vec<_> = registered_commands()
        .into_iter()
        .filter(|name| !allowed.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "commands missing from permissions/*.toml (the app will reject them at runtime): {missing:?}"
    );
}

#[test]
fn the_acl_has_no_stale_command_entries() {
    let registered = registered_commands();
    let stale: Vec<_> = allowed_commands()
        .into_iter()
        .filter(|name| !registered.contains(name))
        .collect();
    assert!(
        stale.is_empty(),
        "permissions/*.toml lists commands that are not registered: {stale:?}"
    );
}
