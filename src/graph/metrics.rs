//! Metric computation functions for the graph executor pipeline.
//!
//! Contains per-language control flow configurations and the compute functions
//! for cyclomatic complexity, cognitive complexity, function length, and
//! comment-to-code ratio. Used by `execute_compute_metric` in `executor.rs`.

use crate::language::Language;
use tree_sitter::Node;

// ── Core types ──────────────────────────────────────────────────────────────

/// Language-specific configuration for control flow analysis.
pub struct ControlFlowConfig {
    /// Node kinds that count as decision points for cyclomatic complexity
    /// (if, for, while, do, catch, case, etc.)
    pub decision_point_kinds: &'static [&'static str],
    /// Node kinds that increment cognitive complexity AND add nesting
    /// (if, for, while, do, switch, catch, etc.)
    pub nesting_increments: &'static [&'static str],
    /// Node kinds that increment cognitive complexity WITHOUT adding nesting
    /// (else if / elif, goto, break-to-label, etc.)
    pub flat_increments: &'static [&'static str],
    /// Logical operator tokens: "&&", "||", "and", "or"
    pub logical_operators: &'static [&'static str],
    /// The node kind for binary expressions containing logical operators
    pub binary_expression_kind: &'static str,
    /// The node kind for ternary/conditional expressions (None if language has none)
    pub ternary_kind: Option<&'static str>,
    /// Node kinds that represent comments
    pub comment_kinds: &'static [&'static str],
}

// ── Compute functions ────────────────────────────────────────────────────────

/// Compute cyclomatic complexity for a function body node.
///
/// CC = 1 + number of decision points + number of logical operators + ternaries
pub fn compute_cyclomatic(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize {
    let mut complexity: usize = 1;

    let mut cursor = body.walk();
    walk_all(body, &mut cursor, &mut |node| {
        let kind = node.kind();

        // Decision points
        if config.decision_point_kinds.contains(&kind) {
            complexity += 1;
        }

        // Ternary expressions
        if let Some(ternary) = config.ternary_kind
            && kind == ternary
        {
            complexity += 1;
        }

        // Logical operators in binary expressions
        if kind == config.binary_expression_kind
            && let Some(op_node) = node.child_by_field_name("operator")
        {
            let op_text = op_node.utf8_text(source).unwrap_or("");
            if config.logical_operators.contains(&op_text) {
                complexity += 1;
            }
        }
    });

    complexity
}

/// Compute cognitive complexity for a function body node.
///
/// Increments for each control flow break. Nesting increments also add
/// a penalty equal to the current nesting depth.
/// Uses stack-based iteration to avoid stack overflow on deeply nested ASTs.
pub fn compute_cognitive(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize {
    let mut score: usize = 0;
    let mut stack: Vec<(Node, usize)> = Vec::new();
    // Seed stack with body's direct children at nesting depth 0 (reverse for L-to-R order)
    let mut cursor = body.walk();
    let children: Vec<_> = body.children(&mut cursor).collect();
    for child in children.into_iter().rev() {
        stack.push((child, 0));
    }
    while let Some((node, nesting)) = stack.pop() {
        let kind = node.kind();
        let (increment, next_nesting) = if config.nesting_increments.contains(&kind) {
            (1 + nesting, nesting + 1)
        } else if config.flat_increments.contains(&kind) {
            (1, nesting)
        } else if config.ternary_kind == Some(kind) {
            (1 + nesting, nesting + 1)
        } else if kind == config.binary_expression_kind {
            if let Some(op_node) = node.child_by_field_name("operator") {
                let op_text = op_node.utf8_text(source).unwrap_or("");
                if config.logical_operators.contains(&op_text) {
                    (1, nesting)
                } else {
                    (0, nesting)
                }
            } else {
                (0, nesting)
            }
        } else {
            (0, nesting)
        };
        score += increment;
        let mut child_cursor = node.walk();
        let node_children: Vec<_> = node.children(&mut child_cursor).collect();
        for child in node_children.into_iter().rev() {
            stack.push((child, next_nesting));
        }
    }
    score
}

/// Compute maximum control flow nesting depth for a function body node.
///
/// Counts how deeply `nesting_increments` nodes are nested within each other.
/// Returns the maximum depth reached (0 = no nesting).
pub fn compute_nesting_depth(body: Node, config: &ControlFlowConfig) -> usize {
    let mut max_depth: usize = 0;
    let mut stack: Vec<(Node, usize)> = Vec::new();
    let mut cursor = body.walk();
    let children: Vec<_> = body.children(&mut cursor).collect();
    for child in children.into_iter().rev() {
        stack.push((child, 0));
    }
    while let Some((node, depth)) = stack.pop() {
        let kind = node.kind();
        let next_depth = if config.nesting_increments.contains(&kind) {
            let new_depth = depth + 1;
            if new_depth > max_depth {
                max_depth = new_depth;
            }
            new_depth
        } else {
            depth
        };
        let mut child_cursor = node.walk();
        let node_children: Vec<_> = node.children(&mut child_cursor).collect();
        for child in node_children.into_iter().rev() {
            stack.push((child, next_depth));
        }
    }
    max_depth
}

/// Count lines and statements in a function body.
///
/// Returns (total_lines, statement_count).
pub fn count_function_lines(body: Node) -> (usize, usize) {
    let start_line = body.start_position().row;
    let end_line = body.end_position().row;
    let total_lines = if end_line >= start_line {
        end_line - start_line + 1
    } else {
        1
    };

    let statement_count = count_statements(body);

    (total_lines, statement_count)
}

fn count_statements(root: Node) -> usize {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if kind.ends_with("_statement")
            || kind.ends_with("_declaration")
            || kind == "expression_statement"
            || kind == "return_statement"
            || kind == "throw_statement"
            || kind == "break_statement"
            || kind == "continue_statement"
        {
            count += 1;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    count
}

/// Compute comment-to-code ratio for the entire file root node.
///
/// Returns (comment_lines, code_lines). Code lines = total non-blank lines minus comment lines.
pub fn compute_comment_ratio(
    root: Node,
    source: &[u8],
    config: &ControlFlowConfig,
) -> (usize, usize) {
    let source_str = std::str::from_utf8(source).unwrap_or("");
    let total_non_blank: usize = source_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    let mut comment_lines: usize = 0;
    let mut cursor = root.walk();
    walk_all(root, &mut cursor, &mut |node| {
        if config.comment_kinds.contains(&node.kind()) {
            let start = node.start_position().row;
            let end = node.end_position().row;
            comment_lines += end - start + 1;
        }
    });

    let code_lines = total_non_blank.saturating_sub(comment_lines);
    (comment_lines, code_lines)
}

/// Walk all descendants of a node, calling `f` on each.
fn walk_all<F: FnMut(Node)>(node: Node, cursor: &mut tree_sitter::TreeCursor, f: &mut F) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        f(current);
        let mut child_cursor = current.walk();
        for child in current.children(&mut child_cursor) {
            stack.push(child);
        }
    }
    // Keep cursor alive for borrow checker
    let _ = cursor;
}

// ── Per-language dispatcher ──────────────────────────────────────────────────

/// Return the `ControlFlowConfig` for a given language.
pub fn control_flow_config_for_language(lang: Language) -> ControlFlowConfig {
    match lang {
        Language::TypeScript | Language::Tsx => ts_config(),
        Language::JavaScript | Language::Jsx => js_config(),
        Language::Rust => rust_config(),
        Language::Python => python_config(),
        Language::Go => go_config(),
        Language::Java => java_config(),
        Language::C => c_config(),
        Language::Cpp => cpp_config(),
        Language::CSharp => csharp_config(),
        Language::Php => php_config(),
    }
}

// ── Per-language config functions ────────────────────────────────────────────

fn ts_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_case",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("ternary_expression"),
        comment_kinds: &["comment"],
    }
}

fn js_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_case",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("ternary_expression"),
        comment_kinds: &["comment"],
    }
}

fn rust_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_arm",
        ],
        nesting_increments: &[
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_expression",
            "closure_expression",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: None,
        comment_kinds: &["line_comment", "block_comment"],
    }
}

fn python_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "elif_clause",
            "for_statement",
            "while_statement",
            "except_clause",
            "with_statement",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "except_clause",
            "with_statement",
        ],
        flat_increments: &["elif_clause", "else_clause"],
        logical_operators: &["and", "or"],
        binary_expression_kind: "boolean_operator",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

fn go_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "expression_case",
            "type_case",
            "communication_case",
            "default_case",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "expression_switch_statement",
            "type_switch_statement",
            "select_statement",
            "func_literal",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: None,
        comment_kinds: &["comment"],
    }
}

fn java_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "enhanced_for_statement",
            "while_statement",
            "do_statement",
            "catch_clause",
            "switch_label",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "enhanced_for_statement",
            "while_statement",
            "do_statement",
            "switch_expression",
            "catch_clause",
        ],
        flat_increments: &[],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("ternary_expression"),
        comment_kinds: &["line_comment", "block_comment"],
    }
}

fn c_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "do_statement",
            "case_statement",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
        ],
        flat_increments: &["else_clause", "goto_statement"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

fn cpp_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_range_loop",
            "while_statement",
            "do_statement",
            "case_statement",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_range_loop",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
            "lambda_expression",
        ],
        flat_increments: &["else_clause", "goto_statement"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

fn csharp_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_section",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

fn php_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "case_statement",
            "catch_clause",
            "else_if_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause", "else_if_clause"],
        logical_operators: &["&&", "||", "and", "or"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

// ── Function body locating helpers ──────────────────────────────────────────

/// Per-language function node kinds for finding function bodies by line number.
pub fn function_node_kinds_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Rust => &["function_item"],
        Language::TypeScript | Language::Tsx => {
            &["function_declaration", "method_definition", "arrow_function", "function"]
        }
        Language::JavaScript | Language::Jsx => {
            &["function_declaration", "method_definition", "arrow_function", "function"]
        }
        Language::Python => &["function_definition"],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C => &["function_definition"],
        Language::Cpp => &["function_definition"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Php => &["function_definition", "method_declaration"],
    }
}

/// Per-language field name for the body child of a function node.
pub fn body_field_for_language(_lang: Language) -> &'static str {
    // All supported languages use "body" as the field name for the function body.
    "body"
}
