use tree_sitter::Node;

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
        if let Some(ternary) = config.ternary_kind {
            if kind == ternary {
                complexity += 1;
            }
        }

        // Logical operators in binary expressions
        if kind == config.binary_expression_kind {
            if let Some(op_node) = node.child_by_field_name("operator") {
                let op_text = op_node.utf8_text(source).unwrap_or("");
                if config.logical_operators.contains(&op_text) {
                    complexity += 1;
                }
            }
        }
    });

    complexity
}

/// Compute cognitive complexity for a function body node.
///
/// Increments for each control flow break. Nesting increments also add
/// a penalty equal to the current nesting depth.
pub fn compute_cognitive(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize {
    let mut score: usize = 0;
    cognitive_walk(body, config, source, 0, &mut score);
    score
}

fn cognitive_walk(
    node: Node,
    config: &ControlFlowConfig,
    source: &[u8],
    nesting: usize,
    score: &mut usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Nesting increments: +1 base + nesting penalty, then recurse with nesting+1
        if config.nesting_increments.contains(&kind) {
            *score += 1 + nesting;
            cognitive_walk(child, config, source, nesting + 1, score);
            continue;
        }

        // Flat increments: +1 base only, no nesting change
        if config.flat_increments.contains(&kind) {
            *score += 1;
            cognitive_walk(child, config, source, nesting, score);
            continue;
        }

        // Ternary
        if let Some(ternary) = config.ternary_kind {
            if kind == ternary {
                *score += 1 + nesting;
                cognitive_walk(child, config, source, nesting + 1, score);
                continue;
            }
        }

        // Logical operator sequences: count switches between different operators
        if kind == config.binary_expression_kind {
            if let Some(op_node) = child.child_by_field_name("operator") {
                let op_text = op_node.utf8_text(source).unwrap_or("");
                if config.logical_operators.contains(&op_text) {
                    *score += 1;
                    cognitive_walk(child, config, source, nesting, score);
                    continue;
                }
            }
        }

        // Otherwise just recurse at same nesting level
        cognitive_walk(child, config, source, nesting, score);
    }
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

fn count_statements(node: Node) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        // Count nodes that end in _statement, _declaration, or _expression_statement
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
        // Recurse into compound/block nodes
        count += count_statements(child);
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn cyclomatic_simple_function() {
        let src = "function foo() { if (x) { } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        // function_declaration -> body is statement_block
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cc = compute_cyclomatic(body, &js_config(), src.as_bytes());
        assert_eq!(cc, 2); // 1 base + 1 if
    }

    #[test]
    fn cyclomatic_with_logical_ops() {
        let src = "function foo() { if (a && b || c) { } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cc = compute_cyclomatic(body, &js_config(), src.as_bytes());
        // 1 base + 1 if + 2 logical ops (&&, ||)
        assert_eq!(cc, 4);
    }

    #[test]
    fn cognitive_nested() {
        let src = "function foo() { if (x) { for (;;) { if (y) { } } } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cog = compute_cognitive(body, &js_config(), src.as_bytes());
        // if: +1 (nesting=0), for: +1+1=2 (nesting=1), inner if: +1+2=3 (nesting=2)
        // total = 1 + 2 + 3 = 6
        assert_eq!(cog, 6);
    }

    #[test]
    fn function_lines_count() {
        let src = "function foo() {\n  let a = 1;\n  let b = 2;\n  return a + b;\n}";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let (lines, _stmts) = count_function_lines(body);
        assert_eq!(lines, 5);
    }

    #[test]
    fn comment_ratio_basic() {
        let src = "// comment\nlet x = 1;\nlet y = 2;\n";
        let tree = parse_js(src);
        let (comment_lines, code_lines) = compute_comment_ratio(
            tree.root_node(),
            src.as_bytes(),
            &js_config(),
        );
        assert_eq!(comment_lines, 1);
        assert_eq!(code_lines, 2);
    }
}
