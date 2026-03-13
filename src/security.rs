use tree_sitter::{Node, Tree};

use crate::language::Language;
use crate::models::SecurityIssue;

struct SecurityConfig {
    call_node_kinds: &'static [&'static str],
    unsafe_functions: &'static [&'static str],
    string_node_kinds: &'static [&'static str],
    variable_decl_kinds: &'static [&'static str],
    assignment_kinds: &'static [&'static str],
}

fn security_config(lang: Language) -> SecurityConfig {
    match lang {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            SecurityConfig {
                call_node_kinds: &["call_expression"],
                unsafe_functions: &["eval", "Function", "execSync", "execFileSync", "spawnSync"],
                string_node_kinds: &["string", "template_string"],
                variable_decl_kinds: &["variable_declarator"],
                assignment_kinds: &[],
            }
        }
        Language::Python => SecurityConfig {
            call_node_kinds: &["call"],
            unsafe_functions: &[
                "eval",
                "exec",
                "compile",
                "os.system",
                "os.popen",
                "subprocess.call",
                "subprocess.run",
                "subprocess.Popen",
            ],
            string_node_kinds: &["string"],
            variable_decl_kinds: &[],
            assignment_kinds: &["assignment"],
        },
        Language::Php => SecurityConfig {
            call_node_kinds: &["function_call_expression"],
            unsafe_functions: &[
                "eval",
                "exec",
                "system",
                "shell_exec",
                "passthru",
                "popen",
                "proc_open",
            ],
            string_node_kinds: &["string", "encapsed_string"],
            variable_decl_kinds: &[],
            assignment_kinds: &["assignment_expression"],
        },
        Language::C => SecurityConfig {
            call_node_kinds: &["call_expression"],
            unsafe_functions: &[
                "system", "popen", "exec", "execl", "execlp", "execle", "execv", "execvp",
            ],
            string_node_kinds: &["string_literal"],
            variable_decl_kinds: &["init_declarator"],
            assignment_kinds: &[],
        },
        Language::Cpp => SecurityConfig {
            call_node_kinds: &["call_expression"],
            unsafe_functions: &[
                "system", "popen", "exec", "execl", "execlp", "execle", "execv", "execvp",
            ],
            string_node_kinds: &["string_literal"],
            variable_decl_kinds: &["init_declarator"],
            assignment_kinds: &[],
        },
        Language::CSharp => SecurityConfig {
            call_node_kinds: &["invocation_expression"],
            unsafe_functions: &["Process.Start"],
            string_node_kinds: &["string_literal", "verbatim_string_literal"],
            variable_decl_kinds: &["variable_declarator"],
            assignment_kinds: &[],
        },
        Language::Rust => SecurityConfig {
            call_node_kinds: &["call_expression", "macro_invocation"],
            unsafe_functions: &["Command::new"],
            string_node_kinds: &["string_literal", "raw_string_literal"],
            variable_decl_kinds: &["let_declaration"],
            assignment_kinds: &[],
        },
        Language::Go => SecurityConfig {
            call_node_kinds: &["call_expression"],
            unsafe_functions: &["exec.Command"],
            string_node_kinds: &["interpreted_string_literal", "raw_string_literal"],
            variable_decl_kinds: &["short_var_declaration", "var_spec"],
            assignment_kinds: &[],
        },
        Language::Java => SecurityConfig {
            call_node_kinds: &["method_invocation"],
            unsafe_functions: &["Runtime.exec", "ProcessBuilder"],
            string_node_kinds: &["string_literal"],
            variable_decl_kinds: &["variable_declarator"],
            assignment_kinds: &[],
        },
    }
}

const SECRET_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "api_secret",
    "secret_key",
    "secret",
    "password",
    "passwd",
    "token",
    "auth_token",
    "access_token",
    "private_key",
    "credential",
    "client_secret",
];

const SQL_PATTERNS: &[&str] = &[
    "select ",
    "insert into",
    "update ",
    "delete from",
    "drop table",
    "drop database",
    "create table",
    "alter table",
];

const HTML_PATTERNS: &[&str] = &[
    "<script", "<iframe", "<form", "<div", "<html", "<body", "onclick=", "onerror=",
];

pub fn extract_security(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
) -> Vec<SecurityIssue> {
    let config = security_config(language);
    let mut issues = Vec::new();
    walk_node(tree.root_node(), source, file_path, &config, &mut issues);
    issues
}

fn walk_node(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &SecurityConfig,
    issues: &mut Vec<SecurityIssue>,
) {
    let kind = node.kind();

    // 1. Check for unsafe calls
    if config.call_node_kinds.contains(&kind) {
        check_unsafe_call(node, source, file_path, config, issues);
    }

    // 2. Check for string risks
    if config.string_node_kinds.contains(&kind) {
        check_string_risk(node, source, file_path, issues);
    }

    // 3. Check for hardcoded secrets
    if config.variable_decl_kinds.contains(&kind) || config.assignment_kinds.contains(&kind) {
        check_hardcoded_secret(node, source, file_path, config, issues);
    }

    // Recurse into children
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            walk_node(child, source, file_path, config, issues);
        }
    }
}

fn check_unsafe_call(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &SecurityConfig,
    issues: &mut Vec<SecurityIssue>,
) {
    // Get the function-position child (first named child typically)
    let func_node = match node.child_by_field_name("function") {
        Some(n) => n,
        None => {
            // For some grammars, try the first named child
            match node.named_child(0) {
                Some(n) => n,
                None => return,
            }
        }
    };

    let func_text = match func_node.utf8_text(source) {
        Ok(t) => t,
        Err(_) => return,
    };

    // Check if the function name matches any unsafe function
    for &unsafe_fn in config.unsafe_functions {
        // Match exact name or dotted suffix (e.g. "os.system" matches "os.system")
        if func_text == unsafe_fn
            || func_text.ends_with(&format!(".{}", unsafe_fn))
            || (unsafe_fn.contains('.') && func_text.ends_with(unsafe_fn))
        {
            issues.push(SecurityIssue {
                file_path: file_path.to_string(),
                issue_type: "unsafe_call".to_string(),
                severity: "high".to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: format!("Call to {}()", unsafe_fn),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
            return;
        }
    }
}

fn check_string_risk(
    node: Node,
    source: &[u8],
    file_path: &str,
    issues: &mut Vec<SecurityIssue>,
) {
    let text = match node.utf8_text(source) {
        Ok(t) => t,
        Err(_) => return,
    };

    let lower = text.to_lowercase();

    // Check SQL patterns
    let has_sql = SQL_PATTERNS.iter().any(|p| lower.contains(p))
        && (lower.contains(" from ") || lower.contains(" into") || lower.contains(" set ") || lower.contains(" table"));

    if has_sql {
        issues.push(SecurityIssue {
            file_path: file_path.to_string(),
            issue_type: "string_risk".to_string(),
            severity: "medium".to_string(),
            line: node.start_position().row as u32,
            column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            description: "String contains inline SQL".to_string(),
            snippet: snippet_text(node, source),
            symbol_name: find_enclosing_symbol(node, source),
        });
        return;
    }

    // Check HTML patterns
    let has_html = HTML_PATTERNS.iter().any(|p| lower.contains(p));
    if has_html {
        issues.push(SecurityIssue {
            file_path: file_path.to_string(),
            issue_type: "string_risk".to_string(),
            severity: "medium".to_string(),
            line: node.start_position().row as u32,
            column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            description: "String contains inline HTML".to_string(),
            snippet: snippet_text(node, source),
            symbol_name: find_enclosing_symbol(node, source),
        });
    }
}

fn check_hardcoded_secret(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &SecurityConfig,
    issues: &mut Vec<SecurityIssue>,
) {
    // Get variable name
    let var_name = extract_variable_name(node, source);
    let var_name = match var_name {
        Some(n) => n,
        None => return,
    };

    let lower_name = var_name.to_lowercase();
    let matches_secret = SECRET_PATTERNS
        .iter()
        .any(|p| lower_name.contains(p));

    if !matches_secret {
        return;
    }

    // Verify the value is a string literal
    if !has_string_value(node, config) {
        return;
    }

    issues.push(SecurityIssue {
        file_path: file_path.to_string(),
        issue_type: "hardcoded_secret".to_string(),
        severity: "high".to_string(),
        line: node.start_position().row as u32,
        column: node.start_position().column as u32,
        end_line: node.end_position().row as u32,
        end_column: node.end_position().column as u32,
        description: format!("Hardcoded secret in variable '{}'", var_name),
        snippet: snippet_text(node, source),
        symbol_name: find_enclosing_symbol(node, source),
    });
}

fn extract_variable_name(node: Node, source: &[u8]) -> Option<String> {
    // Try common field names for variable name
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).ok().map(|s| s.to_string());
    }

    // For Python assignment: left field
    if let Some(left_node) = node.child_by_field_name("left") {
        return left_node.utf8_text(source).ok().map(|s| s.to_string());
    }

    // For init_declarator (C/C++): first named child is the declarator
    if node.kind() == "init_declarator" {
        if let Some(declarator) = node.named_child(0) {
            return declarator.utf8_text(source).ok().map(|s| s.to_string());
        }
    }

    // For let_declaration (Rust): pattern field
    if let Some(pattern) = node.child_by_field_name("pattern") {
        return pattern.utf8_text(source).ok().map(|s| s.to_string());
    }

    None
}

fn has_string_value(node: Node, config: &SecurityConfig) -> bool {
    // Check if value child is a string literal
    if let Some(value_node) = node.child_by_field_name("value") {
        return config.string_node_kinds.contains(&value_node.kind());
    }

    // For Python assignment: right field
    if let Some(right_node) = node.child_by_field_name("right") {
        return config.string_node_kinds.contains(&right_node.kind());
    }

    // For init_declarator: second named child is the value
    if node.kind() == "init_declarator" {
        if let Some(value) = node.named_child(1) {
            return config.string_node_kinds.contains(&value.kind());
        }
    }

    false
}

fn find_enclosing_symbol(node: Node, source: &[u8]) -> String {
    let symbol_kinds = &[
        "function_declaration",
        "function_definition",
        "method_definition",
        "function_item",
        "arrow_function",
        "class_declaration",
        "class_definition",
        "impl_item",
        "method_declaration",
    ];

    let mut current = node.parent();
    while let Some(parent) = current {
        if symbol_kinds.contains(&parent.kind()) {
            if let Some(name_node) = parent.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
        current = parent.parent();
    }
    String::new()
}

fn snippet_text(node: Node, source: &[u8]) -> String {
    let text = node
        .utf8_text(source)
        .unwrap_or("")
        .replace('\n', " ")
        .replace('\r', "");
    if text.len() > 200 {
        format!("{}...", &text[..200])
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn extract(source: &str, lang: Language) -> Vec<SecurityIssue> {
        let mut ts_parser = parser::create_parser(lang).expect("parser");
        let tree = ts_parser.parse(source.as_bytes(), None).expect("parse");
        extract_security(&tree, source.as_bytes(), "test.ts", lang)
    }

    #[test]
    fn detects_eval_call_typescript() {
        let source = r#"function foo() { eval("alert(1)"); }"#;
        let issues = extract(source, Language::TypeScript);
        assert!(issues.iter().any(|i| i.issue_type == "unsafe_call" && i.description.contains("eval")));
    }

    #[test]
    fn detects_sql_in_string() {
        let source = r#"const q = "SELECT * FROM users WHERE id = 1";"#;
        let issues = extract(source, Language::TypeScript);
        assert!(issues.iter().any(|i| i.issue_type == "string_risk" && i.description.contains("SQL")));
    }

    #[test]
    fn detects_hardcoded_secret() {
        let source = r#"const api_key = "sk-1234567890";"#;
        let issues = extract(source, Language::TypeScript);
        assert!(issues.iter().any(|i| i.issue_type == "hardcoded_secret"));
    }

    #[test]
    fn no_false_positive_on_normal_string() {
        let source = r#"const greeting = "hello world";"#;
        let issues = extract(source, Language::TypeScript);
        assert!(issues.is_empty());
    }

    #[test]
    fn detects_python_eval() {
        let source = "result = eval(user_input)\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "unsafe_call" && i.description.contains("eval")));
    }

    #[test]
    fn detects_python_hardcoded_password() {
        let source = "password = \"hunter2\"\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "hardcoded_secret"));
    }

    #[test]
    fn detects_html_in_string() {
        let source = r#"const page = "<script>alert(1)</script>";"#;
        let issues = extract(source, Language::TypeScript);
        assert!(issues.iter().any(|i| i.issue_type == "string_risk" && i.description.contains("HTML")));
    }
}
