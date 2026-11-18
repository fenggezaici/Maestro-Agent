//! Strict allowlist for read-only commands in Code mode.
//!
//! This is an **allow** list, not a deny list: a command runs without approval
//! only when its program, its subcommand and every flag are listed here.
//! Everything else falls back to the normal per-call approval — the previous
//! keyword blacklist was removed precisely because it could not bound
//! `cmd /C` (pipes, indirection, argument variants).
//!
//! A command accepted here is executed **without a shell**, as a parsed argv,
//! so shell quoting/expansion never happens. The parser is still conservative
//! about metacharacters: they are rejected outright so a future change that
//! reintroduces a shell cannot turn an accepted command into a compound one.

/// Same ceiling as the exec adapter: longer strings are never parsed.
const MAX_COMMAND_CHARS: usize = 4096;

/// Characters that only mean something to a shell. Rejected anywhere in the
/// command, including inside quotes, so an accepted command is always a single
/// literal invocation.
const SHELL_METACHARS: &[char] = &[
    '&', '|', ';', '<', '>', '^', '%', '$', '`', '(', ')', '{', '}', '[', ']', '*', '?', '~',
    '!', '#',
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPolicy {
    /// Parsed and allowlisted: run directly (no shell) with these arguments.
    ReadOnly { program: String, args: Vec<String> },
    /// Everything else: suspend for approval and run through the platform shell.
    NeedsApproval,
}

/// Classify one `run_command` string.
pub fn parse(command: &str) -> CommandPolicy {
    if command.len() > MAX_COMMAND_CHARS {
        return CommandPolicy::NeedsApproval;
    }
    let Some(tokens) = tokenize(command) else {
        return CommandPolicy::NeedsApproval;
    };
    let Some(program) = program_name(&tokens[0]) else {
        return CommandPolicy::NeedsApproval;
    };
    let args = &tokens[1..];
    let safe = match program.as_str() {
        "git" => git_args(args),
        "rg" => args_match(args, RG_FLAGS, true) && positionals_are_relative(args),
        "fd" => args_match(args, FD_FLAGS, true) && positionals_are_relative(args),
        "grep" => args_match(args, GREP_FLAGS, true) && positionals_are_relative(args),
        "cat" => args_match(args, CAT_FLAGS, true) && positionals_are_relative(args),
        "head" | "tail" => args_match(args, HEAD_TAIL_FLAGS, true) && positionals_are_relative(args),
        "wc" => args_match(args, WC_FLAGS, true) && positionals_are_relative(args),
        "more" => positionals_are_relative(args) && args.iter().all(|arg| !arg.starts_with('-')),
        "ls" => args_match(args, LS_FLAGS, true) && positionals_are_relative(args),
        "tree" => args_match(args, TREE_FLAGS, true) && positionals_are_relative(args),
        "pwd" | "ver" | "hostname" | "whoami" => args.is_empty(),
        "echo" => true,
        "which" | "where" => where_args(args),
        // Version probes only: `npm -v` is safe, `npm <anything-else>` is not.
        "node" | "npm" | "python" | "python3" | "pip" | "pip3" | "cargo" | "rustc"
        | "go" | "dotnet" | "java" | "javac" | "php" | "ruby" | "deno" | "bun"
        | "pnpm" | "yarn" | "cmake" | "tsc" | "sqlite3" => {
            matches!(args, [arg] if matches!(arg.as_str(), "-v" | "-V" | "--version" | "version"))
        }
        _ => false,
    };
    if safe {
        CommandPolicy::ReadOnly { program, args: args.to_vec() }
    } else {
        CommandPolicy::NeedsApproval
    }
}

/// Convenience wrapper for the mode gate and the audit trail.
pub fn is_read_only_safe(command: &str) -> bool {
    matches!(parse(command), CommandPolicy::ReadOnly { .. })
}

/// Split on spaces, honouring single/double quotes, and refuse anything a
/// shell could reinterpret. No escape sequences: `\"` is not special.
fn tokenize(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        if ch.is_control() {
            return None;
        }
        match quote {
            Some(open) => {
                if ch == open {
                    quote = None;
                } else if SHELL_METACHARS.contains(&ch) {
                    return None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                    started = true;
                } else if ch == ' ' {
                    if started {
                        tokens.push(std::mem::take(&mut current));
                        started = false;
                    }
                } else if SHELL_METACHARS.contains(&ch) {
                    return None;
                } else {
                    current.push(ch);
                    started = true;
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return None;
    }
    Some(tokens)
}

/// The program must be a bare name resolvable through PATH — never a path.
fn program_name(token: &str) -> Option<String> {
    if token.contains(['/', '\\', ':']) || token.contains("..") {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    let name = lower.strip_suffix(".exe").unwrap_or(&lower);
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return None;
    }
    Some(name.to_string())
}

struct FlagSpec {
    name: &'static str,
    takes_value: bool,
}

const fn flag(name: &'static str) -> FlagSpec {
    FlagSpec { name, takes_value: false }
}

const fn valued(name: &'static str) -> FlagSpec {
    FlagSpec { name, takes_value: true }
}

/// Every flag must be listed; positional arguments may appear anywhere (git
/// accepts `git show HEAD --stat`), but a flag is never treated as a value.
fn args_match(args: &[String], flags: &[FlagSpec], allow_positional: bool) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with('-') {
            // Bundled short flags like `-rn` are accepted when every letter
            // is a listed non-valued flag (`grep -rn`, `ls -la`).
            if !arg.starts_with("--") && arg.len() > 2 {
                let bundled = arg[1..]
                    .chars()
                    .all(|letter| {
                        flags.iter().any(|spec| {
                            !spec.takes_value
                                && spec.name.len() == 2
                                && spec.name.starts_with('-')
                                && spec.name.ends_with(letter)
                        })
                    });
                if bundled {
                    index += 1;
                    continue;
                }
            }
            let Some(spec) = flags.iter().find(|spec| spec.name == arg) else {
                return false;
            };
            index += 1;
            if spec.takes_value {
                let Some(value) = args.get(index) else {
                    return false;
                };
                if value.is_empty() || value.starts_with('-') {
                    return false;
                }
                index += 1;
            }
        } else {
            if !allow_positional {
                return false;
            }
            index += 1;
        }
    }
    true
}

const GIT_STATUS: &[FlagSpec] = &[
    flag("--short"),
    flag("-s"),
    flag("--branch"),
    flag("-b"),
    flag("--porcelain"),
    flag("--"),
];
const GIT_DIFF: &[FlagSpec] = &[
    flag("--stat"),
    flag("--shortstat"),
    flag("--numstat"),
    flag("--name-only"),
    flag("--name-status"),
    flag("--cached"),
    flag("--staged"),
    flag("--no-color"),
    flag("--"),
];
const GIT_LOG: &[FlagSpec] = &[
    flag("--oneline"),
    flag("--stat"),
    flag("--graph"),
    flag("--no-color"),
    flag("--name-only"),
    flag("--name-status"),
    flag("-p"),
    valued("-n"),
    valued("--max-count"),
    flag("--"),
];
const GIT_SHOW: &[FlagSpec] = &[
    flag("--stat"),
    flag("--no-color"),
    flag("--name-only"),
    flag("--name-status"),
    flag("--"),
];
const GIT_REV_PARSE: &[FlagSpec] = &[
    flag("--short"),
    flag("--abbrev-ref"),
    flag("--show-toplevel"),
    flag("--is-inside-work-tree"),
    flag("--"),
];
const GIT_LS_FILES: &[FlagSpec] = &[
    flag("--cached"),
    flag("--others"),
    flag("--modified"),
    flag("--deleted"),
    flag("--exclude-standard"),
    flag("--"),
];
const GIT_BLAME: &[FlagSpec] = &[valued("-L"), flag("-n"), flag("--porcelain"), flag("--")];
const GIT_DESCRIBE: &[FlagSpec] = &[flag("--tags"), flag("--always"), flag("--long"), valued("--abbrev")];
const GIT_SHORTLOG: &[FlagSpec] = &[flag("-s"), flag("-n"), flag("--summary"), flag("--")];
const GIT_BRANCH: &[FlagSpec] = &[
    flag("--list"),
    flag("-a"),
    flag("-r"),
    flag("-v"),
    flag("-vv"),
    flag("--all"),
    flag("--remotes"),
    flag("--verbose"),
    flag("--no-color"),
];
const GIT_TAG: &[FlagSpec] = &[
    flag("-l"),
    flag("--list"),
    valued("-n"),
    valued("--sort"),
    valued("--points-at"),
    flag("--"),
];
const GIT_STASH: &[FlagSpec] = &[flag("--stat"), flag("-p"), valued("-n"), flag("--")];
const GIT_GREP: &[FlagSpec] = &[
    flag("-n"),
    flag("-i"),
    flag("-l"),
    flag("-c"),
    flag("-w"),
    flag("-E"),
    flag("-F"),
    flag("--name-only"),
    flag("--"),
];
const RG_FLAGS: &[FlagSpec] = &[
    flag("-n"),
    flag("--line-number"),
    flag("-i"),
    flag("--ignore-case"),
    flag("-l"),
    flag("--files-with-matches"),
    flag("--files"),
    flag("-c"),
    flag("--count"),
    flag("-S"),
    flag("--smart-case"),
    flag("-w"),
    flag("--word-regexp"),
    flag("-F"),
    flag("--fixed-strings"),
    flag("--hidden"),
    flag("--no-heading"),
    valued("-g"),
    valued("--glob"),
    valued("-t"),
    valued("--type"),
    valued("-e"),
    valued("--regexp"),
    valued("-A"),
    valued("-B"),
    valued("-C"),
    valued("-m"),
    valued("--max-count"),
    valued("--max-depth"),
];

fn git_args(args: &[String]) -> bool {
    let Some(subcommand) = args.first() else {
        return false;
    };
    // Global options (`-c`, `--no-pager`, …) can execute or reconfigure git, so
    // the first argument must be a plain subcommand.
    if subcommand.starts_with('-') {
        return false;
    }
    let rest = &args[1..];
    match subcommand.as_str() {
        "status" => args_match(rest, GIT_STATUS, true),
        "diff" => args_match(rest, GIT_DIFF, true),
        "log" => args_match(rest, GIT_LOG, true),
        "show" => args_match(rest, GIT_SHOW, true),
        "rev-parse" => args_match(rest, GIT_REV_PARSE, true),
        "ls-files" => args_match(rest, GIT_LS_FILES, true),
        "blame" => args_match(rest, GIT_BLAME, true),
        "describe" => args_match(rest, GIT_DESCRIBE, false),
        "shortlog" => args_match(rest, GIT_SHORTLOG, true),
        // A positional argument would create/rename/delete a branch.
        "branch" => args_match(rest, GIT_BRANCH, false),
        "config" => git_config(rest),
        // `git tag` only in its list forms: bare, or -l/-n filters — a bare
        // positional would create a tag.
        "tag" => rest.is_empty() || (rest.iter().any(|arg| matches!(arg.as_str(), "-l" | "--list" | "-n"))
            && args_match(rest, GIT_TAG, true)),
        // `git remote` only in read forms.
        "remote" => rest.is_empty()
            || (rest.len() == 1 && rest[0] == "-v")
            || (rest.len() == 2 && rest[0] == "get-url"),
        // `git stash` only the read subcommands.
        "stash" => matches!(rest.first().map(String::as_str), Some("list") | Some("show"))
            && args_match(&rest[1..], GIT_STASH, true),
        "grep" => args_match(rest, GIT_GREP, true),
        _ => false,
    }
}

/// `git config` is only allowed in its read forms: at least one read flag, at
/// most one positional key, and only scope flags besides those.
fn git_config(args: &[String]) -> bool {
    const READ: &[&str] = &["--get", "--get-all", "--list", "-l"];
    const SCOPE: &[&str] = &["--global", "--local", "--system"];
    let mut read = false;
    let mut positional = 0;
    for arg in args {
        if READ.contains(&arg.as_str()) {
            read = true;
        } else if SCOPE.contains(&arg.as_str()) {
            continue;
        } else if arg.starts_with('-') {
            return false;
        } else {
            positional += 1;
            if positional > 1 {
                return false;
            }
        }
    }
    read
}

/// File-reading tools must stay inside the workspace: positional arguments
/// are file paths, and an absolute/`..`/drive-qualified path would let a
/// read-only command exfiltrate data `read_file` is scoped away from
/// (`C:\Windows\…`, `/etc/passwd`). Relative names only.
fn positionals_are_relative(args: &[String]) -> bool {
    args.iter().all(|arg| {
        arg.starts_with('-')
            || (!arg.contains(':')
                && !arg.starts_with('/')
                && !arg.starts_with('\\')
                && !arg.split(['/', '\\']).any(|part| part == ".."))
    })
}

/// `where`/`which` take names/patterns only; `/R` and `-` style switches need
/// approval.
fn where_args(args: &[String]) -> bool {
    !args.is_empty() && args.iter().all(|arg| !arg.starts_with('-') && !arg.starts_with('/'))
}

const FD_FLAGS: &[FlagSpec] = &[
    flag("-H"),
    flag("--hidden"),
    flag("-I"),
    flag("--no-ignore"),
    flag("-s"),
    flag("--case-sensitive"),
    flag("-i"),
    flag("--ignore-case"),
    flag("-g"),
    flag("--glob"),
    flag("-l"),
    flag("--list-details"),
    flag("-a"),
    flag("--absolute-path"),
    flag("--full-path"),
    valued("-t"),
    valued("--type"),
    valued("-e"),
    valued("--extension"),
    valued("-d"),
    valued("--max-depth"),
    valued("-j"),
    valued("--threads"),
];
const GREP_FLAGS: &[FlagSpec] = &[
    flag("-n"),
    flag("-i"),
    flag("-r"),
    flag("-R"),
    flag("-l"),
    flag("-c"),
    flag("-w"),
    flag("-v"),
    flag("-E"),
    flag("-F"),
    flag("--color"),
    valued("-A"),
    valued("-B"),
    valued("-C"),
    valued("-m"),
    valued("--include"),
    valued("--exclude"),
];
const CAT_FLAGS: &[FlagSpec] = &[flag("-n"), flag("-b"), flag("-A"), flag("-E"), flag("-T"), flag("-s")];
const HEAD_TAIL_FLAGS: &[FlagSpec] = &[
    valued("-n"),
    valued("-c"),
    flag("-q"),
    flag("-v"),
    flag("-f"),
    flag("--follow"),
];
const WC_FLAGS: &[FlagSpec] = &[flag("-l"), flag("-w"), flag("-c"), flag("-m"), flag("-L")];
const LS_FLAGS: &[FlagSpec] = &[
    flag("-l"),
    flag("-a"),
    flag("-R"),
    flag("-h"),
    flag("-t"),
    flag("-S"),
    flag("-r"),
    flag("-1"),
    flag("-d"),
    flag("-F"),
    flag("--color"),
];
const TREE_FLAGS: &[FlagSpec] = &[
    flag("/F"),
    flag("/A"),
    flag("-a"),
    flag("-d"),
    flag("-f"),
    flag("-i"),
    flag("--dirsfirst"),
    valued("-L"),
    valued("-I"),
    valued("-P"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn safe(command: &str) -> bool {
        is_read_only_safe(command)
    }

    #[test]
    fn allowlisted_git_read_commands_are_safe() {
        for command in [
            "git status",
            "git status --short",
            "git diff",
            "git diff --stat",
            "git diff --name-only src/a.ts",
            "git diff --cached",
            "git log --oneline -n 20",
            "git log --max-count 5 HEAD",
            "git show HEAD --stat",
            "git rev-parse --abbrev-ref HEAD",
            "git ls-files --modified",
            "git blame -L 1,10 src/a.ts",
            "git describe --tags --always",
            "git shortlog -s HEAD",
            "git branch --list",
            "git branch -a",
            "git config --get user.name",
            "git config --global --get user.name",
            "git config --list",
            "git.exe status",
        ] {
            assert!(safe(command), "{command}");
        }
    }

    #[test]
    fn git_write_or_global_option_commands_need_approval() {
        for command in [
            "git",
            "git checkout .",
            "git commit -m x",
            "git push",
            "git fetch",
            "git reset --hard",
            "git clean -fdx",
            "git stash",
            "git -c core.pager=calc diff",
            "git --no-pager diff",
            "git diff --output=x",
            "git diff --output x",
            "git log --output x",
            "git log --format=%H",
            "git branch -D main",
            "git branch new-branch",
            "git config user.name x",
            "git config --global user.name x",
            "git config --unset user.name",
            "git describe HEAD",
            "git status --short=1",
            "git status -sb extra --stat",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn shell_composition_and_expansion_never_parse_as_safe() {
        for command in [
            "git status && del x",
            "git status; del x",
            "git status | more",
            "git status > out.txt",
            "git status < in.txt",
            "git status\ndel x",
            "git status\tdel x",
            "git status %PATH%",
            "git status ^& del x",
            "git status $(rm -rf .)",
            "git status `rm -rf .`",
            "git status *.rs",
            "git status ~/x",
            "git status !x",
            "git status #x",
            "git status (x)",
            "git status {x}",
            "git status [x]",
            "git status ?",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn program_names_must_be_bare_and_allowlisted() {
        for command in [
            r"C:\Windows\System32\git.exe status",
            "./git status",
            "../git status",
            "tools/git status",
            "npm install",
            "python -c \"print(1)\"",
            "find . -exec rm {} ;",
            "dir",
            "cmd /C git status",
            "powershell -enc AAAA",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn file_reading_tools_only_accept_relative_workspace_paths() {
        for command in [
            "cat src/main.rs",
            "head -n 20 Cargo.toml",
            "tail -f logs/app.log",
            "wc -l src/lib.rs",
            "ls src",
            "tree src",
            "fd -e rs",
            "grep -rn TODO src",
            "echo hello world",
            "pwd",
            "whoami",
            "node --version",
            "python -V",
            "git tag -l",
            "git tag -n 5",
            "git remote -v",
            "git stash list",
            "git grep -n TODO",
        ] {
            assert!(safe(command), "{command}");
        }
        for command in [
            "cat C:/Windows/system.ini",
            "cat /etc/passwd",
            "cat ../outside.txt",
            "head ..\\\\secret.txt",
            "more C:\\\\autoexec.bat",
            "fd -x rm",
            "grep -r x /etc",
            "ls /",
            "tree C:\\\\",
            "echo $PATH",
            "pwd x",
            "node -e print(1)",
            "npm config list",
            "git tag v1.0",
            "git remote add x y",
            "git stash drop",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn rg_and_where_rules_are_flag_scoped() {
        for command in [
            "rg TODO",
            "rg -n TODO src",
            "rg -i --files-with-matches TODO",
            "rg --files",
            "rg -A 2 TODO src",
            "where cmd",
            "where git.exe",
        ] {
            assert!(safe(command), "{command}");
        }
        for command in [
            "rg --pre cat TODO",
            "rg --pre-glob *.rs TODO",
            "rg -z TODO",
            "rg --type-add x:*.x TODO",
            "where",
            "where /R C:\\ x",
            "where -x cmd",
            "rg TODO --pre cat",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn quoted_arguments_group_but_cannot_hide_metacharacters() {
        assert!(safe("git log --oneline -n 20 -- \"main\""));
        assert_eq!(
            parse("git show \"main\""),
            CommandPolicy::ReadOnly { program: "git".into(), args: vec!["show".into(), "main".into()] }
        );
        assert!(!safe("git show \"a|b\""));
        assert!(!safe("git show \"unterminated"));
    }

    #[test]
    fn empty_and_oversized_inputs_are_never_safe() {
        for command in ["", "   ", "\n", "\0"] {
            assert!(!safe(command), "{command:?}");
        }
        assert!(!safe(&format!("git status {}", "x".repeat(5000))));
    }
}
