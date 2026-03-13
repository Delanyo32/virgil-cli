use tree_sitter::{Node, Tree};

use crate::language::Language;
use crate::models::AntipatternIssue;

struct AntipatternConfig {
    language_name: &'static str,
    /// Node kinds that are detected by kind alone (e.g. `non_null_expression`)
    node_kind_checks: &'static [NodeKindCheck],
    /// Enable specific pattern checks
    check_var_keyword: bool,
    check_loose_equality: bool,
    check_unwrap_call: bool,
    check_panic_macro: bool,
    check_bare_except: bool,
    check_mutable_default: bool,
    check_wildcard_import: bool,
    check_empty_catch: bool,
    check_async_void: bool,
    check_ignored_error: bool,
    check_using_namespace_header: bool,
    check_deprecated_mysql: bool,
}

struct NodeKindCheck {
    node_kind: &'static str,
    issue_type: &'static str,
    category: &'static str,
    severity: &'static str,
    description: &'static str,
}

fn antipattern_config(lang: Language) -> AntipatternConfig {
    match lang {
        Language::TypeScript | Language::Tsx => AntipatternConfig {
            language_name: "typescript",
            node_kind_checks: &[
                NodeKindCheck {
                    node_kind: "predefined_type",
                    issue_type: "any_type",
                    category: "type_safety",
                    severity: "medium",
                    description: "Use of 'any' type defeats TypeScript's type safety",
                },
                NodeKindCheck {
                    node_kind: "as_expression",
                    issue_type: "type_assertion",
                    category: "type_safety",
                    severity: "medium",
                    description: "Type assertion bypasses type checking",
                },
                NodeKindCheck {
                    node_kind: "non_null_expression",
                    issue_type: "non_null_assertion",
                    category: "type_safety",
                    severity: "medium",
                    description: "Non-null assertion (!) can mask null reference errors",
                },
            ],
            check_var_keyword: true,
            check_loose_equality: true,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::JavaScript | Language::Jsx => AntipatternConfig {
            language_name: "javascript",
            node_kind_checks: &[],
            check_var_keyword: true,
            check_loose_equality: true,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Rust => AntipatternConfig {
            language_name: "rust",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: true,
            check_panic_macro: true,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Python => AntipatternConfig {
            language_name: "python",
            node_kind_checks: &[
                NodeKindCheck {
                    node_kind: "global_statement",
                    issue_type: "global_statement",
                    category: "maintainability",
                    severity: "medium",
                    description: "Global statement introduces hidden state coupling",
                },
            ],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: true,
            check_mutable_default: true,
            check_wildcard_import: true,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Java => AntipatternConfig {
            language_name: "java",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: true,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::CSharp => AntipatternConfig {
            language_name: "csharp",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: true,
            check_async_void: true,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Go => AntipatternConfig {
            language_name: "go",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: true,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Cpp => AntipatternConfig {
            language_name: "cpp",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: true,

            check_deprecated_mysql: false,
        },
        Language::C => AntipatternConfig {
            language_name: "c",
            node_kind_checks: &[],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: false,
        },
        Language::Php => AntipatternConfig {
            language_name: "php",
            node_kind_checks: &[
                NodeKindCheck {
                    node_kind: "error_suppress_expression",
                    issue_type: "error_suppression",
                    category: "maintainability",
                    severity: "low",
                    description: "Error suppression operator (@) hides errors",
                },
            ],
            check_var_keyword: false,
            check_loose_equality: false,
            check_unwrap_call: false,
            check_panic_macro: false,
            check_bare_except: false,
            check_mutable_default: false,
            check_wildcard_import: false,
            check_empty_catch: false,
            check_async_void: false,
            check_ignored_error: false,
            check_using_namespace_header: false,

            check_deprecated_mysql: true,
        },
    }
}

pub fn extract_antipatterns(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
) -> Vec<AntipatternIssue> {
    let config = antipattern_config(language);
    let mut issues = Vec::new();
    walk_node(tree.root_node(), source, file_path, &config, &mut issues);
    issues
}

fn walk_node(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    let kind = node.kind();

    // 1. Node kind checks (any_type, type_assertion, non_null_assertion, global_statement, error_suppress_expression)
    for check in config.node_kind_checks {
        if kind == check.node_kind {
            // Special case for predefined_type: only flag "any"
            if check.node_kind == "predefined_type" {
                let text = node.utf8_text(source).unwrap_or("");
                if text != "any" {
                    continue;
                }
            }
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: check.issue_type.to_string(),
                category: check.category.to_string(),
                severity: check.severity.to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: check.description.to_string(),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }

    // 2. var keyword check (JS/TS)
    if config.check_var_keyword && kind == "variable_declaration" {
        check_var_keyword(node, source, file_path, config, issues);
    }

    // 3. Loose equality check (JS/TS)
    if config.check_loose_equality && kind == "binary_expression" {
        check_loose_equality(node, source, file_path, config, issues);
    }

    // 4. .unwrap() call check (Rust)
    if config.check_unwrap_call && kind == "call_expression" {
        check_unwrap_call(node, source, file_path, config, issues);
    }

    // 5. panic! macro check (Rust)
    if config.check_panic_macro && kind == "macro_invocation" {
        check_panic_macro(node, source, file_path, config, issues);
    }

    // 6. Bare except check (Python)
    if config.check_bare_except && kind == "except_clause" {
        check_bare_except(node, source, file_path, config, issues);
    }

    // 7. Mutable default argument check (Python)
    if config.check_mutable_default && kind == "default_parameter" {
        check_mutable_default(node, source, file_path, config, issues);
    }

    // 8. Wildcard import check (Python)
    if config.check_wildcard_import && kind == "import_from_statement" {
        check_wildcard_import(node, source, file_path, config, issues);
    }

    // 9. Empty catch check (Java/C#)
    if config.check_empty_catch && kind == "catch_clause" {
        check_empty_catch(node, source, file_path, config, issues);
    }

    // 10. async void check (C#)
    if config.check_async_void && kind == "method_declaration" {
        check_async_void(node, source, file_path, config, issues);
    }

    // 11. Ignored error check (Go)
    if config.check_ignored_error && kind == "short_var_declaration" {
        check_ignored_error(node, source, file_path, config, issues);
    }

    // 12. using namespace in header (C++)
    if config.check_using_namespace_header && kind == "using_declaration" {
        check_using_namespace_header(node, source, file_path, config, issues);
    }

    // 13. Deprecated mysql_* functions (PHP)
    if config.check_deprecated_mysql && kind == "function_call_expression" {
        check_deprecated_mysql(node, source, file_path, config, issues);
    }

    // Recurse into children
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            walk_node(child, source, file_path, config, issues);
        }
    }
}

fn check_var_keyword(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // variable_declaration has a child "var"/"let"/"const" keyword
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            if child.kind() == "var" {
                issues.push(AntipatternIssue {
                    file_path: file_path.to_string(),
                    issue_type: "var_declaration".to_string(),
                    category: "correctness".to_string(),
                    severity: "high".to_string(),
                    language: config.language_name.to_string(),
                    line: node.start_position().row as u32,
                    column: node.start_position().column as u32,
                    end_line: node.end_position().row as u32,
                    end_column: node.end_position().column as u32,
                    description: "Use 'let' or 'const' instead of 'var' to avoid hoisting issues".to_string(),
                    snippet: snippet_text(node, source),
                    symbol_name: find_enclosing_symbol(node, source),
                });
                return;
            }
        }
    }
}

fn check_loose_equality(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // binary_expression has "operator" field
    if let Some(op_node) = node.child_by_field_name("operator") {
        let op = op_node.utf8_text(source).unwrap_or("");
        if op == "==" || op == "!=" {
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: "loose_equality".to_string(),
                category: "correctness".to_string(),
                severity: "high".to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: format!("Use '{}' instead of '{}' to avoid type coercion",
                    if op == "==" { "===" } else { "!==" }, op),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }
}

fn check_unwrap_call(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // call_expression -> function: field_expression -> field: "unwrap"
    if let Some(func_node) = node.child_by_field_name("function") {
        if func_node.kind() == "field_expression" {
            if let Some(field_node) = func_node.child_by_field_name("field") {
                let field_text = field_node.utf8_text(source).unwrap_or("");
                if field_text == "unwrap" {
                    issues.push(AntipatternIssue {
                        file_path: file_path.to_string(),
                        issue_type: "unwrap_call".to_string(),
                        category: "error_handling".to_string(),
                        severity: "high".to_string(),
                        language: config.language_name.to_string(),
                        line: node.start_position().row as u32,
                        column: node.start_position().column as u32,
                        end_line: node.end_position().row as u32,
                        end_column: node.end_position().column as u32,
                        description: "Use '?' operator or 'expect()' instead of 'unwrap()' for better error handling".to_string(),
                        snippet: snippet_text(node, source),
                        symbol_name: find_enclosing_symbol(node, source),
                    });
                }
            }
        }
    }
}

fn check_panic_macro(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // macro_invocation -> macro: identifier "panic"
    if let Some(macro_node) = node.child_by_field_name("macro") {
        let macro_name = macro_node.utf8_text(source).unwrap_or("");
        if macro_name == "panic" {
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: "panic_call".to_string(),
                category: "error_handling".to_string(),
                severity: "high".to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: "Avoid panic!() in library code; return Result instead".to_string(),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }
}

fn check_bare_except(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // except_clause with no exception type — bare `except:`
    // The except_clause node has no named children for the exception type if bare
    let mut has_exception_type = false;
    let count = node.named_child_count();
    for i in 0..count {
        if let Some(child) = node.named_child(i) {
            // If there's a child that is not a block (the handler body), it's an exception type
            if child.kind() != "block" && child.kind() != ":" {
                has_exception_type = true;
                break;
            }
        }
    }

    if !has_exception_type {
        issues.push(AntipatternIssue {
            file_path: file_path.to_string(),
            issue_type: "bare_except".to_string(),
            category: "error_handling".to_string(),
            severity: "high".to_string(),
            language: config.language_name.to_string(),
            line: node.start_position().row as u32,
            column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            description: "Bare 'except:' catches all exceptions including SystemExit and KeyboardInterrupt".to_string(),
            snippet: snippet_text(node, source),
            symbol_name: find_enclosing_symbol(node, source),
        });
    }
}

fn check_mutable_default(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // default_parameter -> value: list/dictionary
    if let Some(value_node) = node.child_by_field_name("value") {
        let vkind = value_node.kind();
        if vkind == "list" || vkind == "dictionary" {
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: "mutable_default".to_string(),
                category: "correctness".to_string(),
                severity: "high".to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: "Mutable default argument is shared across calls; use None and create inside function".to_string(),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }
}

fn check_wildcard_import(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // import_from_statement with wildcard_import child
    let count = node.named_child_count();
    for i in 0..count {
        if let Some(child) = node.named_child(i) {
            if child.kind() == "wildcard_import" {
                issues.push(AntipatternIssue {
                    file_path: file_path.to_string(),
                    issue_type: "wildcard_import".to_string(),
                    category: "maintainability".to_string(),
                    severity: "medium".to_string(),
                    language: config.language_name.to_string(),
                    line: node.start_position().row as u32,
                    column: node.start_position().column as u32,
                    end_line: node.end_position().row as u32,
                    end_column: node.end_position().column as u32,
                    description: "Wildcard import pollutes namespace and makes dependencies unclear".to_string(),
                    snippet: snippet_text(node, source),
                    symbol_name: find_enclosing_symbol(node, source),
                });
                return;
            }
        }
    }
}

fn check_empty_catch(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // catch_clause -> body: block with no named children (empty block)
    if let Some(body_node) = node.child_by_field_name("body") {
        if body_node.kind() == "block" && body_node.named_child_count() == 0 {
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: "empty_catch".to_string(),
                category: "error_handling".to_string(),
                severity: "high".to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: "Empty catch block silently swallows exceptions".to_string(),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }
}

fn check_async_void(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // C# method_declaration: check for async modifier + void return type
    let mut has_async = false;
    let mut has_void_return = false;

    // Check modifiers for "async"
    let count = node.named_child_count();
    for i in 0..count {
        if let Some(child) = node.named_child(i) {
            if child.kind() == "modifier" {
                let text = child.utf8_text(source).unwrap_or("");
                if text == "async" {
                    has_async = true;
                }
            }
        }
    }

    // Check return type for "void"
    if let Some(type_node) = node.child_by_field_name("type") {
        let type_text = type_node.utf8_text(source).unwrap_or("");
        if type_text == "void" {
            has_void_return = true;
        }
    }

    if has_async && has_void_return {
        issues.push(AntipatternIssue {
            file_path: file_path.to_string(),
            issue_type: "async_void".to_string(),
            category: "correctness".to_string(),
            severity: "high".to_string(),
            language: config.language_name.to_string(),
            line: node.start_position().row as u32,
            column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            description: "async void methods cannot be awaited and swallow exceptions; use async Task".to_string(),
            snippet: snippet_text(node, source),
            symbol_name: find_enclosing_symbol(node, source),
        });
    }
}

fn check_ignored_error(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // Go short_var_declaration: look for `_` identifier in the left side
    if let Some(left) = node.child_by_field_name("left") {
        let count = left.named_child_count();
        for i in 0..count {
            if let Some(child) = left.named_child(i) {
                if child.kind() == "identifier" {
                    let text = child.utf8_text(source).unwrap_or("");
                    if text == "_" {
                        issues.push(AntipatternIssue {
                            file_path: file_path.to_string(),
                            issue_type: "ignored_error".to_string(),
                            category: "error_handling".to_string(),
                            severity: "high".to_string(),
                            language: config.language_name.to_string(),
                            line: node.start_position().row as u32,
                            column: node.start_position().column as u32,
                            end_line: node.end_position().row as u32,
                            end_column: node.end_position().column as u32,
                            description: "Ignored error value with '_' can hide failures".to_string(),
                            snippet: snippet_text(node, source),
                            symbol_name: find_enclosing_symbol(node, source),
                        });
                        return;
                    }
                }
            }
        }
    }
}

fn check_using_namespace_header(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // Only flag in header files
    let is_header = file_path.ends_with(".hpp")
        || file_path.ends_with(".hxx")
        || file_path.ends_with(".hh");

    if !is_header {
        return;
    }

    // Check if this is a `using namespace` declaration
    let text = node.utf8_text(source).unwrap_or("");
    if text.contains("namespace") {
        issues.push(AntipatternIssue {
            file_path: file_path.to_string(),
            issue_type: "using_namespace_header".to_string(),
            category: "maintainability".to_string(),
            severity: "low".to_string(),
            language: config.language_name.to_string(),
            line: node.start_position().row as u32,
            column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            description: "'using namespace' in header files pollutes the global namespace for all includers".to_string(),
            snippet: snippet_text(node, source),
            symbol_name: find_enclosing_symbol(node, source),
        });
    }
}

fn check_deprecated_mysql(
    node: Node,
    source: &[u8],
    file_path: &str,
    config: &AntipatternConfig,
    issues: &mut Vec<AntipatternIssue>,
) {
    // PHP function_call_expression with mysql_* name
    if let Some(func_node) = node.child_by_field_name("function") {
        let func_text = func_node.utf8_text(source).unwrap_or("");
        if func_text.starts_with("mysql_") {
            issues.push(AntipatternIssue {
                file_path: file_path.to_string(),
                issue_type: "deprecated_mysql".to_string(),
                category: "maintainability".to_string(),
                severity: "medium".to_string(),
                language: config.language_name.to_string(),
                line: node.start_position().row as u32,
                column: node.start_position().column as u32,
                end_line: node.end_position().row as u32,
                end_column: node.end_position().column as u32,
                description: format!("{}() is deprecated; use mysqli or PDO instead", func_text),
                snippet: snippet_text(node, source),
                symbol_name: find_enclosing_symbol(node, source),
            });
        }
    }
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

    fn extract(source: &str, lang: Language) -> Vec<AntipatternIssue> {
        let mut ts_parser = parser::create_parser(lang).expect("parser");
        let tree = ts_parser.parse(source.as_bytes(), None).expect("parse");
        extract_antipatterns(&tree, source.as_bytes(), "test.ts", lang)
    }

    #[test]
    fn detects_var_declaration() {
        let source = "var x = 1;";
        let issues = extract(source, Language::JavaScript);
        assert!(issues.iter().any(|i| i.issue_type == "var_declaration"));
    }

    #[test]
    fn no_var_for_let() {
        let source = "let x = 1;";
        let issues = extract(source, Language::JavaScript);
        assert!(!issues.iter().any(|i| i.issue_type == "var_declaration"));
    }

    #[test]
    fn detects_loose_equality() {
        let source = "if (x == 1) {}";
        let issues = extract(source, Language::JavaScript);
        assert!(issues.iter().any(|i| i.issue_type == "loose_equality"));
    }

    #[test]
    fn no_loose_for_strict() {
        let source = "if (x === 1) {}";
        let issues = extract(source, Language::JavaScript);
        assert!(!issues.iter().any(|i| i.issue_type == "loose_equality"));
    }

    #[test]
    fn detects_any_type_typescript() {
        let source = "let x: any = 1;";
        let issues = extract(source, Language::TypeScript);
        assert!(issues.iter().any(|i| i.issue_type == "any_type"));
    }

    #[test]
    fn detects_unwrap_rust() {
        let source = "fn main() { let x = foo().unwrap(); }";
        let issues = extract(source, Language::Rust);
        assert!(issues.iter().any(|i| i.issue_type == "unwrap_call"));
    }

    #[test]
    fn detects_panic_macro_rust() {
        let source = r#"fn main() { panic!("oh no"); }"#;
        let issues = extract(source, Language::Rust);
        assert!(issues.iter().any(|i| i.issue_type == "panic_call"));
    }

    #[test]
    fn detects_bare_except_python() {
        let source = "try:\n    pass\nexcept:\n    pass\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "bare_except"));
    }

    #[test]
    fn no_bare_except_with_type() {
        let source = "try:\n    pass\nexcept ValueError:\n    pass\n";
        let issues = extract(source, Language::Python);
        assert!(!issues.iter().any(|i| i.issue_type == "bare_except"));
    }

    #[test]
    fn detects_mutable_default_python() {
        let source = "def foo(items=[]):\n    pass\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "mutable_default"));
    }

    #[test]
    fn detects_wildcard_import_python() {
        let source = "from os import *\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "wildcard_import"));
    }

    #[test]
    fn detects_global_statement_python() {
        let source = "def foo():\n    global x\n    x = 1\n";
        let issues = extract(source, Language::Python);
        assert!(issues.iter().any(|i| i.issue_type == "global_statement"));
    }

    #[test]
    fn detects_ignored_error_go() {
        let source = "package main\nfunc main() {\n    _, _ := foo()\n}\n";
        // Note: Go parser may need valid Go syntax
        let mut ts_parser = parser::create_parser(Language::Go).expect("parser");
        let tree = ts_parser.parse(source.as_bytes(), None).expect("parse");
        let issues = extract_antipatterns(&tree, source.as_bytes(), "test.go", Language::Go);
        assert!(issues.iter().any(|i| i.issue_type == "ignored_error"));
    }
}
