use tree_sitter::{Node, Tree};

use crate::language::Language;
use crate::models::{ComplexityInfo, SymbolInfo, SymbolKind};

/// Language-specific node kinds that contribute to complexity metrics.
struct ComplexityConfig {
    /// Node kinds that are branching decision points (if, for, while, etc.)
    branching: &'static [&'static str],
    /// Node kinds that increase nesting depth for cognitive complexity
    nesting: &'static [&'static str],
    /// Node kind for logical operator expressions (binary_expression, boolean_operator, etc.)
    logical_op_node: &'static str,
    /// Operators considered logical (&&, ||, ??, and, or)
    logical_operators: &'static [&'static str],
    /// Node kind for ternary/conditional expressions (if any)
    ternary: Option<&'static str>,
    /// Node kinds that are non-nesting increments for cognitive (else, elif)
    non_nesting: &'static [&'static str],
}

fn complexity_config(lang: Language) -> ComplexityConfig {
    match lang {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            ComplexityConfig {
                branching: &[
                    "if_statement",
                    "for_statement",
                    "for_in_statement",
                    "while_statement",
                    "do_statement",
                    "switch_case",
                    "catch_clause",
                ],
                nesting: &[
                    "if_statement",
                    "for_statement",
                    "for_in_statement",
                    "while_statement",
                    "do_statement",
                    "switch_case",
                    "catch_clause",
                ],
                logical_op_node: "binary_expression",
                logical_operators: &["&&", "||", "??"],
                ternary: Some("ternary_expression"),
                non_nesting: &["else_clause"],
            }
        }
        Language::Rust => ComplexityConfig {
            branching: &[
                "if_expression",
                "for_expression",
                "while_expression",
                "loop_expression",
                "match_arm",
            ],
            nesting: &[
                "if_expression",
                "for_expression",
                "while_expression",
                "loop_expression",
                "match_arm",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: None,
            non_nesting: &["else_clause"],
        },
        Language::Python => ComplexityConfig {
            branching: &[
                "if_statement",
                "elif_clause",
                "for_statement",
                "while_statement",
                "except_clause",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "while_statement",
                "except_clause",
            ],
            logical_op_node: "boolean_operator",
            logical_operators: &["and", "or"],
            ternary: Some("conditional_expression"),
            non_nesting: &["elif_clause", "else_clause"],
        },
        Language::Go => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "expression_case",
                "type_case",
                "communication_case",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "expression_case",
                "type_case",
                "communication_case",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: None,
            non_nesting: &[],
        },
        Language::Java => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "enhanced_for_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "switch_block_statement_group",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "enhanced_for_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "switch_block_statement_group",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: Some("ternary_expression"),
            non_nesting: &["else"],
        },
        Language::C => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "while_statement",
                "do_statement",
                "case_statement",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "while_statement",
                "do_statement",
                "case_statement",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: Some("conditional_expression"),
            non_nesting: &["else_clause"],
        },
        Language::Cpp => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "for_range_loop",
                "while_statement",
                "do_statement",
                "case_statement",
                "catch_clause",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "for_range_loop",
                "while_statement",
                "do_statement",
                "case_statement",
                "catch_clause",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: Some("conditional_expression"),
            non_nesting: &["else_clause"],
        },
        Language::CSharp => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "for_each_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "switch_section",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "for_each_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "switch_section",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||"],
            ternary: Some("conditional_expression"),
            non_nesting: &["else_clause"],
        },
        Language::Php => ComplexityConfig {
            branching: &[
                "if_statement",
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "case_statement",
            ],
            nesting: &[
                "if_statement",
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
                "catch_clause",
                "case_statement",
            ],
            logical_op_node: "binary_expression",
            logical_operators: &["&&", "||", "and", "or"],
            ternary: Some("conditional_expression"),
            non_nesting: &["else_clause", "else_if_clause"],
        },
    }
}

/// Check if a symbol kind should get individual complexity scores.
fn is_complexity_relevant(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::ArrowFunction
    )
}

/// Calculate cyclomatic complexity for a subtree.
/// Base = 1, +1 per decision point.
fn calculate_cyclomatic(node: Node, source: &[u8], config: &ComplexityConfig) -> u32 {
    let mut complexity = 0u32;
    let kind = node.kind();

    // Check branching decision points
    if config.branching.contains(&kind) {
        complexity += 1;
    }

    // Check logical operators
    if kind == config.logical_op_node {
        if let Some(op_node) = node.child_by_field_name("operator") {
            let op_text = op_node
                .utf8_text(source)
                .unwrap_or("");
            if config.logical_operators.contains(&op_text) {
                complexity += 1;
            }
        }
    }

    // Check ternary
    if let Some(ternary_kind) = config.ternary {
        if kind == ternary_kind {
            complexity += 1;
        }
    }

    // Recurse into children
    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            complexity += calculate_cyclomatic(child, source, config);
        }
    }

    complexity
}

/// Calculate cognitive complexity for a subtree (Sonar-style).
fn calculate_cognitive(node: Node, source: &[u8], config: &ComplexityConfig, nesting: u32) -> u32 {
    let mut complexity = 0u32;
    let kind = node.kind();

    // Nesting constructs: increment = 1 + current_nesting_depth
    if config.nesting.contains(&kind) {
        complexity += 1 + nesting;

        // Recurse children with incremented nesting
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                complexity += calculate_cognitive(child, source, config, nesting + 1);
            }
        }
        return complexity;
    }

    // Non-nesting constructs (else, elif): increment = 1, same nesting
    if config.non_nesting.contains(&kind) {
        complexity += 1;

        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                complexity += calculate_cognitive(child, source, config, nesting);
            }
        }
        return complexity;
    }

    // Logical operators: flat +1 each
    if kind == config.logical_op_node {
        if let Some(op_node) = node.child_by_field_name("operator") {
            let op_text = op_node
                .utf8_text(source)
                .unwrap_or("");
            if config.logical_operators.contains(&op_text) {
                complexity += 1;
            }
        }
    }

    // Ternary: nesting construct
    if let Some(ternary_kind) = config.ternary {
        if kind == ternary_kind {
            complexity += 1 + nesting;

            let child_count = node.child_count();
            for i in 0..child_count {
                if let Some(child) = node.child(i) {
                    complexity += calculate_cognitive(child, source, config, nesting + 1);
                }
            }
            return complexity;
        }
    }

    // Regular node: recurse with same nesting
    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            complexity += calculate_cognitive(child, source, config, nesting);
        }
    }

    complexity
}

/// Extract complexity metrics for all relevant symbols in a file.
pub fn extract_complexity(
    tree: &Tree,
    source: &[u8],
    symbols: &[SymbolInfo],
    file_path: &str,
    language: Language,
) -> Vec<ComplexityInfo> {
    let config = complexity_config(language);
    let root = tree.root_node();

    symbols
        .iter()
        .filter(|sym| is_complexity_relevant(sym.kind))
        .filter_map(|sym| {
            // Find the AST node for this symbol by its position
            let start_point = tree_sitter::Point::new(sym.start_line as usize, sym.start_column as usize);
            let end_point = tree_sitter::Point::new(sym.end_line as usize, sym.end_column as usize);

            let node = root.descendant_for_point_range(start_point, end_point)?;

            let cyclomatic = 1 + calculate_cyclomatic(node, source, &config);
            let cognitive = calculate_cognitive(node, source, &config, 0);
            let line_count = sym.end_line.saturating_sub(sym.start_line).saturating_add(1);

            Some(ComplexityInfo {
                file_path: file_path.to_string(),
                symbol_name: sym.name.clone(),
                symbol_kind: sym.kind.to_string(),
                start_line: sym.start_line,
                end_line: sym.end_line,
                line_count,
                cyclomatic_complexity: cyclomatic,
                cognitive_complexity: cognitive,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn parse_and_extract(source: &str, lang: Language) -> Vec<ComplexityInfo> {
        let mut ts_parser = parser::create_parser(lang).expect("parser");
        let tree = ts_parser.parse(source.as_bytes(), None).expect("parse");
        let config = complexity_config(lang);
        let _ = config;

        // Extract symbols using the language module
        let query = crate::languages::compile_symbol_query(lang).expect("query");
        let symbols =
            crate::languages::extract_symbols(&tree, source.as_bytes(), &query, "test.rs", lang);

        extract_complexity(&tree, source.as_bytes(), &symbols, "test.rs", lang)
    }

    #[test]
    fn simple_function_cyclomatic_one() {
        let source = "fn hello() { println!(\"hi\"); }";
        let results = parse_and_extract(source, Language::Rust);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cyclomatic_complexity, 1);
        assert_eq!(results[0].cognitive_complexity, 0);
    }

    #[test]
    fn function_with_if_cyclomatic_two() {
        let source = r#"fn check(x: i32) -> bool {
            if x > 0 {
                true
            } else {
                false
            }
        }"#;
        let results = parse_and_extract(source, Language::Rust);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cyclomatic_complexity, 2); // base 1 + 1 if
    }

    #[test]
    fn function_with_nested_if_cognitive() {
        let source = r#"fn check(x: i32, y: i32) -> bool {
            if x > 0 {
                if y > 0 {
                    true
                } else {
                    false
                }
            } else {
                false
            }
        }"#;
        let results = parse_and_extract(source, Language::Rust);
        assert_eq!(results.len(), 1);
        // Cyclomatic: base 1 + 2 ifs = 3
        assert_eq!(results[0].cyclomatic_complexity, 3);
        // Cognitive: outer if = 1+0=1, inner if = 1+1=2, else(inner) = 1, else(outer) = 1 => 5
        assert_eq!(results[0].cognitive_complexity, 5);
    }

    #[test]
    fn typescript_function_with_logical_ops() {
        let source = r#"function check(a: boolean, b: boolean, c: boolean): boolean {
            return a && b || c;
        }"#;
        let results = parse_and_extract(source, Language::TypeScript);
        assert_eq!(results.len(), 1);
        // Cyclomatic: base 1 + 2 logical ops = 3
        assert_eq!(results[0].cyclomatic_complexity, 3);
    }

    #[test]
    fn python_function_with_for_loop() {
        let source = "def count(items):\n    total = 0\n    for item in items:\n        total += 1\n    return total\n";
        let results = parse_and_extract(source, Language::Python);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cyclomatic_complexity, 2); // base 1 + 1 for
    }

    #[test]
    fn skips_non_function_symbols() {
        let source = r#"
struct Foo {
    x: i32,
}

fn bar() {}
"#;
        let results = parse_and_extract(source, Language::Rust);
        // Should only have bar(), not Foo
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "bar");
    }
}
