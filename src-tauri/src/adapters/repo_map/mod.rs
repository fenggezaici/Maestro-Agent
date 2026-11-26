//! Repo map: the workspace compressed into a token-budgeted symbol index.
//! tree-sitter parses each source file into definitions (signatures only, no
//! bodies) and reference identifiers; a SQLite cache keyed on file mtime makes
//! repeat scans incremental; cross-file reference counts rank what matters.
//!
//! The model consumes this through the `repo_map` read-only tool — a stranger
//! to the codebase stops blind-walking the file tree and navigates by the map
//! instead (Aider validates the approach at scale). Deliberately out of scope:
//! embeddings/vector indexes, LSP, file watchers, more languages.

mod languages;

pub use languages::{FileSymbols, SymbolDef};

use std::collections::HashMap;
use std::path::Path;

use crate::adapters::db::Database;
use crate::domain::AppResult;

/// 扫描上限——超出即截断并在输出头部声明（地图是检索辅助，不是全量索引）。
const MAX_FILES: usize = 2000;
/// 单文件上限：生成代码/压缩产物超此尺寸不贡献符号（误报收益比极低）。
const MAX_FILE_BYTES: u64 = 512 * 1024;
pub const DEFAULT_BUDGET: usize = 2000;
pub const MIN_BUDGET: usize = 500;
pub const MAX_BUDGET: usize = 8000;
/// token 估算：4 字符/token 是 Anthropic/OpenAI 通用的保守近似。
const CHARS_PER_TOKEN: usize = 4;

/// 单文件的缓存行——db 层往返的传输结构。
pub struct CacheRow {
    pub mtime: i64,
    pub symbols: FileSymbols,
}

/// 工作区符号扫描：mtime 增量解析 + 引用计数排名 + 预算渲染。
/// `db` 为 None（内存态/测试）时每次全量解析——功能等价，只损失速度。
pub async fn generate(
    db: Option<&Database>,
    root: &Path,
    query: Option<&str>,
    paths: Option<&[String]>,
    budget: usize,
) -> AppResult<String> {
    let budget = budget.clamp(MIN_BUDGET, MAX_BUDGET);
    let files = collect_source_files(root)?;
    if files.is_empty() {
        return Ok("（工作区内没有可索引的源文件——支持 .py/.js/.jsx/.ts/.tsx/.rs）".into());
    }
    let truncated_scan = files.len() >= MAX_FILES;

    // 1) 缓存对账：mtime 未变的直接复用，变化的进解析队列。
    let root_key = root.to_string_lossy().replace('\\', "/");
    let cached: HashMap<String, (i64, FileSymbols)> = match db {
        Some(db) => db.repo_map_cache(&root_key).await.unwrap_or_default(),
        None => HashMap::new(),
    };
    let mut symbols: HashMap<String, FileSymbols> = HashMap::new();
    let mut stale: Vec<(String, i64)> = Vec::new();
    for (rel, mtime) in &files {
        match cached.get(rel.as_str()) {
            Some((cached_mtime, cached_symbols)) if *cached_mtime == *mtime => {
                symbols.insert(rel.clone(), clone_symbols(cached_symbols));
            }
            _ => stale.push((rel.clone(), *mtime)),
        }
    }

    // 2) 失效文件批量解析——CPU 密集，进 spawn_blocking 不阻塞运行时。
    let root_owned = root.to_path_buf();
    let stale_count = stale.len();
    let parsed: Vec<(String, i64, FileSymbols)> = if stale.is_empty() {
        Vec::new()
    } else {
        tokio::task::spawn_blocking(move || {
            stale
                .iter()
                .filter_map(|(rel, mtime)| {
                    let full = root_owned.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                    let bytes = std::fs::read(&full).ok()?;
                    let source = String::from_utf8_lossy(&bytes);
                    let lang = languages::lang_for_path(rel)?;
                    Some((rel.clone(), *mtime, languages::extract(lang, &source)))
                })
                .collect()
        })
        .await
        .map_err(|_| "符号解析任务失败".to_string())?
    };
    for (rel, mtime, file_symbols) in &parsed {
        symbols.insert(rel.clone(), clone_symbols(file_symbols));
        if let Some(db) = db {
            let _ = db
                .repo_map_upsert(&root_key, rel, *mtime, file_symbols)
                .await;
        }
    }
    // 消失文件的缓存行顺手清理（同 workspace 作用域）。
    if let Some(db) = db {
        let live: Vec<String> = files.iter().map(|(rel, _)| rel.clone()).collect();
        let _ = db.repo_map_prune(&root_key, &live).await;
    }

    // 3) 排名 + 渲染。
    Ok(render(&symbols, query, paths, budget, truncated_scan, stale_count))
}

fn clone_symbols(symbols: &FileSymbols) -> FileSymbols {
    FileSymbols {
        defs: symbols.defs.clone(),
        refs: symbols.refs.clone(),
    }
}

/// 工作区内全部支持语言的源文件（workspace 相对路径 + mtime 秒）。
/// 复用 fs 适配器的 junk-list 遍历——node_modules/target/dist 不进索引。
fn collect_source_files(root: &Path) -> AppResult<Vec<(String, i64)>> {
    let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
    let mut all = Vec::new();
    crate::adapters::fs::walk_source_files(&base, &mut all);
    let mut out: Vec<(String, i64)> = all
        .into_iter()
        .filter(|rel| languages::lang_for_path(rel).is_some())
        .filter_map(|rel| {
            let full = base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let meta = std::fs::metadata(&full).ok()?;
            if meta.len() > MAX_FILE_BYTES {
                return None;
            }
            let mtime = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some((rel, mtime))
        })
        .collect();
    out.sort();
    out.truncate(MAX_FILES);
    Ok(out)
}

/// 渲染：引用计数加权 → 文件级排序 → 预算内逐行输出。
/// query 命中（路径或符号名子串，大小写不敏感）的文件强制优先；
/// paths 显式指定的文件必出现（只要它们在索引内）。
fn render(
    symbols: &HashMap<String, FileSymbols>,
    query: Option<&str>,
    paths: Option<&[String]>,
    budget: usize,
    truncated_scan: bool,
    parsed_count: usize,
) -> String {
    // 符号名 → 跨文件引用数（排除定义文件自身——自己引用自己不构成重要性）。
    let mut ref_counts: HashMap<&str, usize> = HashMap::new();
    let def_names: std::collections::HashSet<&str> = symbols
        .values()
        .flat_map(|file| file.defs.iter().map(|def| def.name.as_str()))
        .collect();
    for (path, file) in symbols {
        let local_defs: std::collections::HashSet<&str> =
            file.defs.iter().map(|def| def.name.as_str()).collect();
        for reference in &file.refs {
            if def_names.contains(reference.as_str()) && !local_defs.contains(reference.as_str()) {
                *ref_counts.entry(reference.as_str()).or_default() += 1;
            }
        }
        let _ = path;
    }

    // 文件权重 = 其最重磅符号的引用数 + 次重磅的半值（头部集中，长尾递减）。
    let weight = |file: &FileSymbols| -> usize {
        let mut scores: Vec<usize> = file
            .defs
            .iter()
            .map(|def| ref_counts.get(def.name.as_str()).copied().unwrap_or(0))
            .collect();
        scores.sort_unstable_by(|a, b| b.cmp(a));
        scores.first().copied().unwrap_or(0) * 2 + scores.get(1).copied().unwrap_or(0)
    };

    let query_lower = query.map(|q| q.to_ascii_lowercase());
    let matches_query = |path: &str, file: &FileSymbols| -> bool {
        query_lower.as_ref().is_some_and(|q| {
            path.to_ascii_lowercase().contains(q.as_str())
                || file.defs.iter().any(|def| def.name.to_ascii_lowercase().contains(q.as_str()))
        })
    };
    let explicit: std::collections::HashSet<&str> = paths
        .map(|list| list.iter().map(|p| p.as_str()).collect())
        .unwrap_or_default();

    let mut entries: Vec<(&String, &FileSymbols, usize, bool)> = symbols
        .iter()
        .filter(|(_, file)| !file.defs.is_empty())
        .map(|(path, file)| {
            let forced = explicit.contains(path.as_str()) || matches_query(path, file);
            (path, file, weight(file), forced)
        })
        .collect();
    // forced 优先，其后按权重降序、路径升序稳定。
    entries.sort_by(|a, b| {
        b.3.cmp(&a.3)
            .then(b.2.cmp(&a.2))
            .then(a.0.cmp(b.0))
    });

    let token_cap = budget * CHARS_PER_TOKEN;
    let mut used = 0usize;
    let mut body = String::new();
    let mut files_shown = 0usize;
    let mut truncated_files = 0usize;
    for (path, file, _, _) in &entries {
        // 文件头 + 至少一行符号的代价超标即收尾。
        let header = format!("{path}:\n");
        if used + header.len() + 40 > token_cap && files_shown > 0 {
            truncated_files += 1;
            continue;
        }
        let mut file_block = header.clone();
        let mut file_used = header.len();
        let mut lines = 0usize;
        // 文件内：权重降序，同权按行号（阅读动线）。
        let mut defs: Vec<&SymbolDef> = file.defs.iter().collect();
        defs.sort_by(|a, b| {
            let wa = ref_counts.get(a.name.as_str()).copied().unwrap_or(0);
            let wb = ref_counts.get(b.name.as_str()).copied().unwrap_or(0);
            wb.cmp(&wa).then(a.line.cmp(&b.line))
        });
        for def in defs {
            let refs = ref_counts.get(def.name.as_str()).copied().unwrap_or(0);
            let suffix = if refs > 0 { format!("  ← {refs} refs") } else { String::new() };
            let line = format!("  {} ({})\n", def.signature, suffix);
            if used + file_used + line.len() > token_cap && lines > 0 {
                break;
            }
            file_block.push_str(&line);
            file_used += line.len();
            lines += 1;
        }
        if lines == 0 {
            continue;
        }
        body.push_str(&file_block);
        used += file_used;
        files_shown += 1;
    }

    let total_defs: usize = symbols.values().map(|file| file.defs.len()).sum();
    let mut header = format!(
        "仓库地图 · {} 文件 {} 符号（本次解析 {parsed_count}，缓存复用 {}）",
        symbols.len(),
        total_defs,
        symbols.len().saturating_sub(parsed_count)
    );
    if truncated_scan {
        header.push_str(&format!(" · 扫描超 {MAX_FILES} 文件上限已截断"));
    }
    if truncated_files > 0 {
        header.push_str(&format!(" · 预算外省略 {truncated_files} 个文件"));
    }
    header.push_str("\n\n");
    if body.is_empty() {
        return format!("{header}（无符号可显示——调整 query/paths 或增大 budget）");
    }
    format!("{header}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(defs: Vec<(&str, &'static str, &str, usize)>, refs: Vec<&str>) -> FileSymbols {
        FileSymbols {
            defs: defs
                .into_iter()
                .map(|(name, kind, signature, line)| SymbolDef {
                    name: name.into(),
                    kind: kind.into(),
                    signature: signature.into(),
                    line,
                })
                .collect(),
            refs: refs.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn hot_symbols_rank_first_within_budget() {
        let mut symbols = HashMap::new();
        // hot.rs 定义 execute；a/b/c 三个文件引用它。cold.rs 无人问津。
        symbols.insert("src/hot.rs".into(), file(vec![("execute", "fn", "pub fn execute()", 10)], vec![]));
        symbols.insert("src/cold.rs".into(), file(vec![("unused_helper", "fn", "fn unused_helper()", 1)], vec![]));
        for user in ["src/a.rs", "src/b.rs", "src/c.rs"] {
            symbols.insert(user.into(), file(vec![("local", "fn", "fn local()", 1)], vec!["execute"]));
        }
        let out = render(&symbols, None, None, DEFAULT_BUDGET, false, 0);
        let hot_pos = out.find("src/hot.rs").expect("hot.rs 在地图中");
        let cold_pos = out.find("src/cold.rs").expect("cold.rs 在地图中");
        assert!(hot_pos < cold_pos, "高引用文件应排前");
        assert!(out.contains("← 3 refs"), "引用计数标注：{out}");
    }

    #[test]
    fn budget_truncates_with_honest_note() {
        let mut symbols = HashMap::new();
        for index in 0..40 {
            symbols.insert(
                format!("src/file{index:02}.rs"),
                file(vec![("somefun", "fn", "pub fn somefun_with_long_signature(x: usize)", 1)], vec![]),
            );
        }
        let out = render(&symbols, None, None, MIN_BUDGET, false, 0);
        assert!(out.contains("预算外省略"), "截断声明：{out}");
        // 输出规模受预算约束（字符级粗略校验——4 字符/token）。
        assert!(out.len() < MIN_BUDGET * CHARS_PER_TOKEN + 400, "len={}", out.len());
    }

    #[test]
    fn query_pulls_matching_files_forward() {
        let mut symbols = HashMap::new();
        symbols.insert("src/unrelated.rs".into(), file(vec![("alpha", "fn", "fn alpha()", 1)], vec!["helper"]));
        symbols.insert("src/deep/target.rs".into(), file(vec![("beta", "fn", "fn beta()", 1)], vec![]));
        let out = render(&symbols, Some("target"), None, DEFAULT_BUDGET, false, 0);
        let target_pos = out.find("src/deep/target.rs").expect("query 命中文件在列");
        let unrelated_pos = out.find("src/unrelated.rs").expect("未命中文件在列");
        assert!(target_pos < unrelated_pos, "query 命中应前置");
    }

    #[tokio::test]
    async fn end_to_end_generate_over_real_files() {
        // 全链路：walk → 解析 → 排名 → 渲染（无 db 的内存模式）。
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/core.py"),
            "class Engine:
    def run(self):
        pass

def bootstrap():
    return Engine()
",
        ).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn execute() -> bool { true }
fn hidden() {}
",
        ).unwrap();
        // 不支持的语言与 node_modules 不进来。
        std::fs::write(dir.path().join("README.md"), "# nothing").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        std::fs::write(dir.path().join("node_modules/pkg/index.js"), "function noise() {}").unwrap();

        let out = generate(None, dir.path(), None, None, DEFAULT_BUDGET).await.unwrap();
        assert!(out.contains("Engine"), "python class 入图：{out}");
        assert!(out.contains("bootstrap"), "python fn 入图：{out}");
        assert!(out.contains("execute"), "rust fn 入图：{out}");
        assert!(!out.contains("noise"), "node_modules 不入索引：{out}");
        assert!(out.contains("仓库地图"), "头部统计：{out}");
        // query 过滤链路。
        let filtered = generate(None, dir.path(), Some("lib.rs"), None, DEFAULT_BUDGET).await.unwrap();
        let lib_pos = filtered.find("src/lib.rs").expect("query 命中在列");
        let core_pos = filtered.find("src/core.py").expect("未命中也在列");
        assert!(lib_pos < core_pos);
    }

    #[test]
    fn files_without_defs_are_not_rendered() {
        let mut symbols = HashMap::new();
        symbols.insert("src/empty.rs".into(), file(vec![], vec!["x"]));
        let out = render(&symbols, None, None, DEFAULT_BUDGET, false, 0);
        assert!(!out.contains("src/empty.rs:"));
    }
}
