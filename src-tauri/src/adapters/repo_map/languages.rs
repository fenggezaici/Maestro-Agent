//! Per-language tree-sitter extraction: definitions (name/kind/signature/line)
//! and reference identifiers. Grammar knowledge lives here alone — the cache,
//! ranking and rendering layers only see the normalized `FileSymbols`.
//!
//! Extraction is deliberately syntactic, not semantic: no type resolution, no
//! import graph. References are identifier-name matches across files — an
//! approximation that ranks well in practice (Aider validates the approach at
//! scale) and never claims precision it does not have.

use tree_sitter::{Language, Node, Parser};

/// 单个符号定义：签名截取到 body 起点（不含函数体），≤200 字符防爆。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SymbolDef {
    pub name: String,
    pub kind: std::borrow::Cow<'static, str>,
    pub signature: String,
    /// 1-based 行号——与编辑器/grep 口径一致。
    pub line: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct FileSymbols {
    pub defs: Vec<SymbolDef>,
    /// 文件内全部标识符去重集——跨文件引用计数的事实源。
    pub refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lang {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
}

/// 扩展名 → 语言。jsx/mjs/cjs 归 JavaScript；tsx 单独（grammar 不同）。
pub fn lang_for_path(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "py" => Some(Lang::Python),
        "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
        "ts" | "mts" | "cts" => Some(Lang::TypeScript),
        "tsx" => Some(Lang::Tsx),
        "rs" => Some(Lang::Rust),
        _ => None,
    }
}

fn language(lang: Lang) -> Language {
    match lang {
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
    }
}

/// 定义节点类型 → 符号类别标签。返回 None 表示非定义节点。
fn def_kind(lang: Lang, node_kind: &str) -> Option<&'static str> {
    match lang {
        Lang::Python => match node_kind {
            "function_definition" => Some("def"),
            "class_definition" => Some("class"),
            _ => None,
        },
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => match node_kind {
            "function_declaration" | "function" => Some("function"),
            "class_declaration" | "abstract_class_declaration" => Some("class"),
            "method_definition" => Some("method"),
            "interface_declaration" => Some("interface"),
            "type_alias_declaration" => Some("type"),
            "enum_declaration" => Some("enum"),
            _ => None,
        },
        Lang::Rust => match node_kind {
            "function_item" => Some("fn"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "trait_item" => Some("trait"),
            "impl_item" => Some("impl"),
            "type_item" => Some("type"),
            "mod_item" => Some("mod"),
            "const_item" => Some("const"),
            "static_item" => Some("static"),
            _ => None,
        },
    }
}

/// 引用统计的标识符节点类型。type_identifier 覆盖 Rust/TS 的类型位置；
/// property_identifier 计入方法引用（obj.method() 的 method）。
fn is_identifier_node(lang: Lang, node_kind: &str) -> bool {
    match node_kind {
        "identifier" => true,
        "type_identifier" => matches!(lang, Lang::Rust | Lang::TypeScript | Lang::Tsx),
        "property_identifier" => matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx),
        "field_identifier" => lang == Lang::Rust,
        _ => false,
    }
}

const MAX_SIGNATURE_CHARS: usize = 200;

/// 签名文本：节点起点到 body 起点（无 body 取首行），去尾部分隔符。
fn signature_text(source: &str, node: Node) -> String {
    let body_start = node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let raw = &source[node.start_byte().min(source.len())..body_start.min(source.len())];
    // impl_item 的 body 就是其全部意义所在（impl Trait for Type）——
    // header 部分正好是 "impl Trait for Type"，语义正确。
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim_end_matches(['{', ':', ' '])
        .trim_end()
        .to_string();
    if trimmed.is_empty() {
        // 无 body 的单行声明（enum/type alias 等）——退回首行。
        let first = source[node.start_byte().min(source.len())..node.end_byte().min(source.len())]
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(['{', ':', ' '])
            .to_string();
        return truncate_chars(&first, MAX_SIGNATURE_CHARS);
    }
    truncate_chars(&trimmed, MAX_SIGNATURE_CHARS)
}

fn truncate_chars(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cap).collect();
    out.push('…');
    out
}

/// JS/TS 的箭头函数/函数表达式常赋给变量——`const f = () => {}` 的语义
/// 定义在 variable_declarator 上。向上找一层拿变量名。
fn declarator_name(node: Node, source: &str) -> Option<String> {
    if node.kind() == "variable_declarator" {
        return node
            .child_by_field_name("name")
            .map(|name| source[name.byte_range()].to_string());
    }
    None
}

/// 解析单个文件，提取 defs + refs。解析失败（语法错误文件常见）返回
/// 部分结果而非错误——repo map 是检索辅助，不因单文件失败而整体哑火。
pub fn extract(lang: Lang, source: &str) -> FileSymbols {
    let mut parser = Parser::new();
    if parser.set_language(&language(lang)).is_err() {
        return FileSymbols::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FileSymbols::default();
    };
    let mut out = FileSymbols::default();
    let mut refs = std::collections::HashSet::new();
    let root = tree.root_node();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        // 定义提取。decorated_definition（Python）/ export_statement（JS/TS）
        // 是包装节点——不进 defs，但继续下探（子定义在栈遍历中自然命中）。
        if let Some(kind) = def_kind(lang, node.kind()) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = source[name_node.byte_range()].to_string();
                out.defs.push(SymbolDef {
                    name,
                    kind: kind.into(),
                    signature: signature_text(source, node),
                    line: node.start_position().row + 1,
                });
            } else if node.kind() == "impl_item" {
                // impl 块无名——以 type 字段为名（impl Foo / impl Trait for Foo）。
                if let Some(type_node) = node.child_by_field_name("type") {
                    let name = source[type_node.byte_range()].to_string();
                    out.defs.push(SymbolDef {
                        name: format!("impl {name}"),
                        kind: kind.into(),
                        signature: signature_text(source, node),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        // JS/TS：`const f = () => {}` / `const f = function() {}` —— variable
        // declarator 的 value 是函数表达式时按函数定义收录。
        if matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx) && node.kind() == "variable_declarator" {
            if let Some(value) = node.child_by_field_name("value") {
                if matches!(value.kind(), "arrow_function" | "function" | "function_expression") {
                    if let Some(name) = declarator_name(node, source) {
                        out.defs.push(SymbolDef {
                            name,
                            kind: "function".into(),
                            signature: signature_text(source, node),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }
        if is_identifier_node(lang, node.kind()) {
            refs.insert(source[node.byte_range()].to_string());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    // 稳定的输出顺序：按行号排序（栈遍历是逆序的）。
    out.defs.sort_by_key(|def| def.line);
    // 嵌套定义去重（同名同行只留一份——decorated/export 包装不重复计数，
    // 因为包装节点自身不是 def_kind；但 Rust impl 内方法与 trait 内签名
    // 可能同名同行号并列，去重保持输出紧凑）。
    out.defs.dedup_by(|a, b| a.line == b.line && a.name == b.name && a.kind == b.kind);
    out.refs = refs.into_iter().collect();
    out.refs.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_defs_and_refs() {
        let source = r#"
import os

class Server:
    def handle(self, request):
        return self.process(request)

def process(data):
    return os.path.join(data)

CONSTANT = 42
"#;
        let symbols = extract(Lang::Python, source);
        let names: Vec<_> = symbols.defs.iter().map(|d| (d.name.as_str(), d.kind.as_ref())).collect();
        assert!(names.contains(&("Server", "class")), "{names:?}");
        assert!(names.contains(&("handle", "def")), "{names:?}");
        assert!(names.contains(&("process", "def")), "{names:?}");
        // 签名不含函数体。
        let process_def = symbols.defs.iter().find(|d| d.name == "process").unwrap();
        assert_eq!(process_def.signature, "def process(data)");
        // 引用集含跨文件可匹配的标识符。
        assert!(symbols.refs.contains(&"os".to_string()));
        assert!(symbols.refs.contains(&"process".to_string()));
    }

    #[test]
    fn rust_defs_cover_types_and_impls() {
        let source = r#"
pub struct ToolRouter { field: u32 }
pub enum Mode { Plan, Code }
pub trait Execute {
    fn run(&self) -> bool;
}
impl ToolRouter {
    pub fn execute(&self, input: &str) -> Result<String, Error> {
        todo!()
    }
}
fn helper() {}
const LIMIT: usize = 10;
"#;
        let symbols = extract(Lang::Rust, source);
        let kinds: Vec<_> = symbols.defs.iter().map(|d| (d.name.as_str(), d.kind.as_ref())).collect();
        assert!(kinds.contains(&("ToolRouter", "struct")), "{kinds:?}");
        assert!(kinds.contains(&("Mode", "enum")), "{kinds:?}");
        assert!(kinds.contains(&("Execute", "trait")), "{kinds:?}");
        assert!(kinds.iter().any(|(n, k)| n.starts_with("impl") && *k == "impl"), "{kinds:?}");
        assert!(kinds.contains(&("execute", "fn")), "{kinds:?}");
        assert!(kinds.contains(&("helper", "fn")), "{kinds:?}");
        assert!(kinds.contains(&("LIMIT", "const")), "{kinds:?}");
        let exec = symbols.defs.iter().find(|d| d.name == "execute" && d.kind == "fn").unwrap();
        assert!(exec.signature.starts_with("pub fn execute"), "{}", exec.signature);
        assert!(!exec.signature.contains("todo!"), "签名不含 body：{}", exec.signature);
    }

    #[test]
    fn typescript_defs_include_interfaces_and_arrow_consts() {
        let source = r#"
export interface Props { value: string }
type Handler = (input: string) => void;
export class Panel {
    render() { return null; }
}
const compute = (x: number): number => x * 2;
function legacy() {}
"#;
        let symbols = extract(Lang::TypeScript, source);
        let kinds: Vec<_> = symbols.defs.iter().map(|d| (d.name.as_str(), d.kind.as_ref())).collect();
        assert!(kinds.contains(&("Props", "interface")), "{kinds:?}");
        assert!(kinds.contains(&("Handler", "type")), "{kinds:?}");
        assert!(kinds.contains(&("Panel", "class")), "{kinds:?}");
        assert!(kinds.contains(&("render", "method")), "{kinds:?}");
        assert!(kinds.contains(&("compute", "function")), "{kinds:?}");
        assert!(kinds.contains(&("legacy", "function")), "{kinds:?}");
    }

    #[test]
    fn javascript_plain_functions() {
        let source = "function alpha() { return 1; }\nconst beta = function() {};\nclass Gamma {}";
        let symbols = extract(Lang::JavaScript, source);
        let names: Vec<_> = symbols.defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"alpha"), "{names:?}");
        assert!(names.contains(&"beta"), "{names:?}");
        assert!(names.contains(&"Gamma"), "{names:?}");
    }

    #[test]
    fn broken_source_yields_partial_not_panic() {
        let symbols = extract(Lang::Rust, "fn broken( { }}}}{");
        // 不 panic 即合格；defs 可有可无（错误恢复由 tree-sitter 决定）。
        let _ = symbols;
    }

    #[test]
    fn lang_detection() {
        assert_eq!(lang_for_path("a/b.py"), Some(Lang::Python));
        assert_eq!(lang_for_path("x.TSX".to_ascii_lowercase().as_str()), Some(Lang::Tsx));
        assert_eq!(lang_for_path("x.jsx"), Some(Lang::JavaScript));
        assert_eq!(lang_for_path("x.rs"), Some(Lang::Rust));
        assert_eq!(lang_for_path("x.md"), None);
    }
}
