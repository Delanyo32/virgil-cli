use std::sync::Arc;

use anyhow::Result;
use petgraph::Direction;
use petgraph::visit::EdgeRef;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{Pipeline, PipelineContext};
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const SQL_METHODS: &[&str] = &["execute", "executemany", "executescript"];

pub struct SqlInjectionPipeline {
    call_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL injection risks: f-strings, format(), %, or concatenation in execute() calls"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(call_node)) =
                (fn_node, args_node, call_node)
            {
                if fn_node.kind() != "attribute" {
                    continue;
                }

                let attr = fn_node
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source));
                let method_name = match attr {
                    Some(name) if SQL_METHODS.contains(&name) => name,
                    _ => continue,
                };

                // Check the first argument for unsafe patterns
                if let Some(first_arg) = args_node.named_child(0) {
                    let kind = first_arg.kind();

                    // Check for f-string: string node with interpolation children
                    let is_fstring = kind == "string" && has_interpolation(first_arg);

                    let (pattern, msg) = if is_fstring {
                        (
                            "sql_fstring",
                            format!(
                                "`.{method_name}()` with f-string — use parameterized queries instead"
                            ),
                        )
                    } else {
                        match kind {
                            // %-format: binary_operator with % on a string
                            "binary_operator" => {
                                let text = node_text(first_arg, source);
                                if text.contains('%') {
                                    (
                                        "sql_percent_format",
                                        format!(
                                            "`.{method_name}()` with %-formatting — use parameterized queries instead"
                                        ),
                                    )
                                } else if text.contains('+') {
                                    (
                                        "sql_concat",
                                        format!(
                                            "`.{method_name}()` with string concatenation — use parameterized queries instead"
                                        ),
                                    )
                                } else {
                                    continue;
                                }
                            }
                            // .format() call: the arg itself is a call with attribute .format
                            "call" => {
                                let arg_text = node_text(first_arg, source);
                                if arg_text.contains(".format(") {
                                    (
                                        "sql_format",
                                        format!(
                                            "`.{method_name}()` with .format() — use parameterized queries instead"
                                        ),
                                    )
                                } else {
                                    continue;
                                }
                            }
                            _ => continue,
                        }
                    };

                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: msg,
                        snippet: extract_snippet(source, call_node, 1),
                    });
                }
            }
        }

        findings
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        let mut ts_findings = self.check(ctx.tree, ctx.source, ctx.file_path);

        // When graph is available, add graph-based findings and filter tree-sitter ones
        if let Some(graph) = ctx.graph {
            let graph_findings = check_sql_injection_via_graph(graph, ctx.file_path);

            // Filter sql_fstring findings — suppress when no external taint
            ts_findings.retain(|f| {
                if f.pattern != "sql_fstring" {
                    return true;
                }
                // Check if the f-string uses only constants (ALL_CAPS identifiers)
                if is_constant_interpolation(ctx.tree, ctx.source, f.line) {
                    return false; // suppress — safe constant interpolation
                }
                // Check if the enclosing function receives external input via graph
                if !function_has_external_input(graph, ctx.file_path, f.line) {
                    return false; // suppress — no external input flows here
                }
                true
            });

            // Merge graph findings
            ts_findings.extend(graph_findings);
        }

        ts_findings
    }
}

/// Query the CodeGraph for unsanitized taint paths to SQL sink calls in this file.
fn check_sql_injection_via_graph(graph: &CodeGraph, file_path: &str) -> Vec<AuditFinding> {
    let mut findings = Vec::new();

    // Find all symbol nodes in this file
    for sym_idx in graph.graph.node_indices() {
        let (sym_file, sym_name, start_line) = match &graph.graph[sym_idx] {
            NodeWeight::Symbol {
                file_path: fp,
                name,
                start_line,
                ..
            } => (fp.as_str(), name.as_str(), *start_line),
            _ => continue,
        };

        if sym_file != file_path {
            continue;
        }

        // Check if this symbol has any incoming FlowsTo edges (tainted data flows in)
        let has_taint_flow = graph
            .graph
            .edges_directed(sym_idx, Direction::Incoming)
            .any(|e| matches!(e.weight(), EdgeWeight::FlowsTo));

        if !has_taint_flow {
            continue;
        }

        // Check if there's a SanitizedBy edge (data was sanitized)
        let is_sanitized = graph
            .graph
            .edges_directed(sym_idx, Direction::Outgoing)
            .any(|e| matches!(e.weight(), EdgeWeight::SanitizedBy { .. }));

        if is_sanitized {
            continue;
        }

        // This symbol has unsanitized taint flow — it's a potential SQL injection
        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start_line,
            column: 1,
            severity: "error".to_string(),
            pipeline: "sql_injection".to_string(),
            pattern: "sql_taint_flow".to_string(),
            message: format!(
                "Unsanitized data flow to SQL sink via '{}' — use parameterized queries",
                sym_name
            ),
            snippet: String::new(),
        });
    }

    findings
}

/// Checks if a Python `string` node contains `interpolation` children (i.e., is an f-string).
fn has_interpolation(node: tree_sitter::Node) -> bool {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i)
            && child.kind() == "interpolation"
        {
            return true;
        }
    }
    false
}

/// Check if an f-string at `finding_line` (1-indexed) only interpolates ALL_CAPS constants.
///
/// Walks the tree to find the call node at the given line, locates the f-string argument,
/// and checks whether every interpolated expression is an ALL_CAPS identifier (e.g. `TABLE_NAME`).
/// Returns `true` only if ALL interpolated variables are constants.
fn is_constant_interpolation(tree: &Tree, source: &[u8], finding_line: u32) -> bool {
    let target_row = finding_line.saturating_sub(1); // convert to 0-indexed
    let root = tree.root_node();

    // Find the deepest node at this line that is a `call` node
    let call_node = match find_call_at_line(root, target_row) {
        Some(n) => n,
        None => return false,
    };

    // Get the argument_list
    let args_node = match call_node.child_by_field_name("arguments") {
        Some(n) => n,
        None => return false,
    };

    // Get the first named argument (the SQL string)
    let first_arg = match args_node.named_child(0) {
        Some(n) => n,
        None => return false,
    };

    // Must be a string with interpolations
    if first_arg.kind() != "string" || !has_interpolation(first_arg) {
        return false;
    }

    // Check all interpolated expressions
    let mut found_any = false;
    for i in 0..first_arg.named_child_count() {
        if let Some(child) = first_arg.named_child(i) {
            if child.kind() == "interpolation" {
                found_any = true;
                // The expression inside the interpolation
                let expr = match child.named_child(0) {
                    Some(e) => e,
                    None => return false,
                };
                if expr.kind() != "identifier" {
                    return false;
                }
                let name = node_text(expr, source);
                // Check ALL_CAPS pattern: at least one letter, only uppercase letters + underscores + digits
                if !is_all_caps_constant(name) {
                    return false;
                }
            }
        }
    }

    found_any
}

/// Returns true if the name matches ALL_CAPS constant convention:
/// only uppercase ASCII letters, digits, and underscores, with at least one letter.
fn is_all_caps_constant(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let has_letter = name.chars().any(|c| c.is_ascii_uppercase());
    let all_valid = name
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    has_letter && all_valid
}

/// Walk the tree to find a `call` node whose start row matches `target_row`.
/// Prefers the shallowest (outermost) call at the matching line.
fn find_call_at_line(node: tree_sitter::Node, target_row: u32) -> Option<tree_sitter::Node> {
    // If this node doesn't contain the target row at all, skip
    if (node.start_position().row as u32) > target_row
        || (node.end_position().row as u32) < target_row
    {
        return None;
    }

    // If this node itself is a call at the right line, return it (prefer outermost)
    if node.kind() == "call" && node.start_position().row as u32 == target_row {
        return Some(node);
    }

    // Otherwise, recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(found) = find_call_at_line(child, target_row) {
                return Some(found);
            }
        }
    }

    None
}

/// Check if the enclosing function of a finding receives external input via the graph.
///
/// Finds the Symbol node in the graph whose line range contains `finding_line`,
/// then checks if that symbol has any incoming `FlowsTo` edges from `ExternalSource` nodes
/// (directly or via one hop through intermediate nodes).
fn function_has_external_input(graph: &CodeGraph, file_path: &str, finding_line: u32) -> bool {
    // Find the enclosing symbol: the closest Symbol node in this file
    // whose start_line <= finding_line <= end_line
    let mut enclosing_idx = None;
    let mut best_range = u32::MAX; // narrowest enclosing range wins

    for idx in graph.graph.node_indices() {
        if let NodeWeight::Symbol {
            file_path: fp,
            start_line,
            end_line,
            ..
        } = &graph.graph[idx]
        {
            if fp == file_path && *start_line <= finding_line && finding_line <= *end_line {
                let range = end_line - start_line;
                if range < best_range {
                    best_range = range;
                    enclosing_idx = Some(idx);
                }
            }
        }
    }

    let sym_idx = match enclosing_idx {
        Some(idx) => idx,
        None => return false, // no enclosing function found
    };

    // Check direct incoming FlowsTo edges from ExternalSource nodes
    for edge in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge.weight(), EdgeWeight::FlowsTo) {
            let source = edge.source();
            if matches!(graph.graph[source], NodeWeight::ExternalSource { .. }) {
                return true;
            }
            // One-hop transitive: check if the source node itself has incoming
            // FlowsTo from an ExternalSource
            for inner_edge in graph.graph.edges_directed(source, Direction::Incoming) {
                if matches!(inner_edge.weight(), EdgeWeight::FlowsTo)
                    && matches!(graph.graph[inner_edge.source()], NodeWeight::ExternalSource { .. })
                {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_fstring_in_execute() {
        let src = "cursor.execute(f\"SELECT * FROM users WHERE id = {id}\")";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_fstring");
    }

    #[test]
    fn detects_percent_format() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = %s\" % user_id)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_percent_format");
    }

    #[test]
    fn detects_format_call() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = {}\".format(user_id))";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_format");
    }

    #[test]
    fn detects_concatenation() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = \" + user_id)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_concat");
    }

    #[test]
    fn ignores_parameterized_query() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = ?\", (user_id,))";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    // --- check_with_context tests ---

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        use std::collections::HashMap;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SqlInjectionPipeline::new().unwrap();
        let id_counts = HashMap::new();
        let graph = CodeGraph::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: Some(&graph),
        };
        pipeline.check_with_context(&ctx)
    }

    #[test]
    fn context_suppresses_constant_fstring() {
        let src = "cursor.execute(f\"SELECT * FROM {TABLE_NAME}\")";
        let findings = parse_and_check_with_context(src);
        // TABLE_NAME is ALL_CAPS → constant → suppress
        assert!(
            findings.is_empty(),
            "Expected constant f-string to be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn context_suppresses_no_external_input() {
        // With an empty graph (no FlowsTo edges), the enclosing function
        // has no external input, so sql_fstring should be suppressed
        let src = "cursor.execute(f\"SELECT * FROM users WHERE id = {user_id}\")";
        let findings = parse_and_check_with_context(src);
        // user_id is not ALL_CAPS, but the empty graph has no external input → suppress
        assert!(
            findings.is_empty(),
            "Expected f-string to be suppressed when graph has no external input, got: {:?}",
            findings
        );
    }

    #[test]
    fn context_still_flags_variable_fstring() {
        // When graph shows external input flowing to the function, the finding should persist
        use crate::graph::SourceKind;
        use crate::models::SymbolKind;
        use std::collections::HashMap;

        let src = "def handle(request):\n    user_id = request.args.get('id')\n    cursor.execute(f\"SELECT * FROM users WHERE id = {user_id}\")";

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();

        let pipeline = SqlInjectionPipeline::new().unwrap();
        let id_counts = HashMap::new();

        // Build a graph with an ExternalSource flowing into the function
        let mut graph = CodeGraph::new();
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "handle".to_string(),
            kind: SymbolKind::Function,
            file_path: "test.py".to_string(),
            start_line: 1,
            end_line: 3,
            exported: false,
        });
        let ext_idx = graph.graph.add_node(NodeWeight::ExternalSource {
            kind: SourceKind::UserInput,
            file_path: "test.py".to_string(),
            line: 2,
        });
        graph.graph.add_edge(ext_idx, sym_idx, EdgeWeight::FlowsTo);

        let ctx = PipelineContext {
            tree: &tree,
            source: src.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: Some(&graph),
        };
        let findings = pipeline.check_with_context(&ctx);
        // Tree-sitter sql_fstring is kept (external input present) + graph sql_taint_flow is added
        assert!(
            findings.iter().any(|f| f.pattern == "sql_fstring"),
            "Expected sql_fstring finding to be kept when external input is present"
        );
    }

    #[test]
    fn context_still_flags_percent_format() {
        // Non-fstring patterns should pass through unchanged
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = %s\" % user_id)";
        let findings = parse_and_check_with_context(src);
        assert_eq!(findings.len(), 1, "Expected percent format to pass through");
        assert_eq!(findings[0].pattern, "sql_percent_format");
    }
}
