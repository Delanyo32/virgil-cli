use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{Pipeline, PipelineContext};

use super::primitives::{
    compile_call_query, compile_function_def_query, extract_snippet, find_capture_index, node_text,
};

pub struct PathTraversalPipeline {
    call_query: Arc<Query>,
    fn_query: Arc<Query>,
}

impl PathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
            fn_query: compile_function_def_query()?,
        })
    }
}

impl Pipeline for PathTraversalPipeline {
    fn name(&self) -> &str {
        "path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: open() or os.path.join() with function parameters"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let params_idx = find_capture_index(&self.fn_query, "params");
        let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        // Find all function definitions
        let mut fn_cursor = QueryCursor::new();
        let mut fn_matches = fn_cursor.matches(&self.fn_query, tree.root_node(), source);

        while let Some(fn_m) = fn_matches.next() {
            let params_node = fn_m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let body_node = fn_m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_body_idx)
                .map(|c| c.node);
            let _fn_name_node = fn_m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);

            if let (Some(params), Some(body)) = (params_node, body_node) {
                // Collect parameter names
                let params_text = node_text(params, source);
                if params_text == "()" || params_text == "(self)" {
                    continue;
                }

                let param_names: Vec<&str> = collect_param_names(params, source);
                if param_names.is_empty() {
                    continue;
                }

                // Search for calls within the function body
                let mut call_cursor = QueryCursor::new();
                call_cursor.set_byte_range(body.byte_range());
                let mut call_matches =
                    call_cursor.matches(&self.call_query, tree.root_node(), source);

                while let Some(cm) = call_matches.next() {
                    let fn_node = cm
                        .captures
                        .iter()
                        .find(|c| c.index as usize == fn_expr_idx)
                        .map(|c| c.node);
                    let call_args = cm
                        .captures
                        .iter()
                        .find(|c| c.index as usize == args_idx)
                        .map(|c| c.node);
                    let call_node = cm
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(fn_node), Some(call_args), Some(call_node)) =
                        (fn_node, call_args, call_node)
                    {
                        let (is_path_op, pattern) = is_path_operation(fn_node, source);
                        if !is_path_op {
                            continue;
                        }

                        // Check if any argument uses a function parameter
                        if args_contain_param(call_args, source, &param_names) {
                            let start = call_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: pattern.to_string(),
                                message: "file path operation with function parameter — validate path to prevent traversal".to_string(),
                                snippet: extract_snippet(source, call_node, 1),
                            });
                        }
                    }
                }
            }
        }

        findings
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        // Suppress all findings in test files
        if is_test_file(ctx.file_path) {
            return Vec::new();
        }

        let base = self.check(ctx.tree, ctx.source, ctx.file_path);

        base.into_iter()
            .filter(|f| !is_path_validated(ctx.tree, ctx.source, f.line))
            .collect()
    }
}

/// Check if the file path indicates a test file.
fn is_test_file(file_path: &str) -> bool {
    file_path.contains("/tests/")
        || file_path.contains("/test/")
        || file_path.contains("test_")
        || file_path.ends_with("_test.py")
}

/// Validation function names that indicate a path has been sanitized.
const VALIDATION_NAMES: &[&str] = &[
    "abspath",
    "realpath",
    "normpath",
    "basename",
    "resolve",
    "secure_filename",
];

/// Check if the function enclosing the finding at `finding_line` (1-indexed)
/// contains path validation/sanitization calls before that line.
fn is_path_validated(tree: &Tree, source: &[u8], finding_line: u32) -> bool {
    let finding_row = finding_line.saturating_sub(1); // convert to 0-indexed

    // Find the enclosing function_definition by walking up from the finding location
    let root = tree.root_node();
    let Some(finding_node) = root.descendant_for_point_range(
        tree_sitter::Point::new(finding_row as usize, 0),
        tree_sitter::Point::new(finding_row as usize, 0),
    ) else {
        return false;
    };

    // Walk up to find the enclosing function_definition
    let mut current = Some(finding_node);
    let mut fn_body = None;
    while let Some(node) = current {
        if node.kind() == "function_definition" {
            fn_body = node.child_by_field_name("body");
            break;
        }
        current = node.parent();
    }

    let Some(body) = fn_body else {
        return false;
    };

    // Scan the function body for validation patterns before the finding line
    scan_for_validation(body, source, finding_row)
}

/// Recursively scan nodes in the function body for validation patterns
/// that appear before `finding_row` (0-indexed).
fn scan_for_validation(node: tree_sitter::Node, source: &[u8], finding_row: u32) -> bool {
    // Only consider nodes on or before the finding line (validation may wrap the operation)
    if node.start_position().row as u32 > finding_row {
        return false;
    }

    match node.kind() {
        "call" => {
            // Check if this call is a validation function
            if let Some(fn_expr) = node.child_by_field_name("function") {
                let fn_text = node_text(fn_expr, source);
                for name in VALIDATION_NAMES {
                    if fn_text.contains(name) {
                        return true;
                    }
                }
            }
        }
        "if_statement" => {
            // Check if the condition contains ".." or "../" string checks
            if let Some(condition) = node.child_by_field_name("condition") {
                let cond_text = node_text(condition, source);
                if cond_text.contains("\"..\"") || cond_text.contains("\"../\"") {
                    return true;
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i)
            && scan_for_validation(child, source, finding_row)
        {
            return true;
        }
    }

    false
}

fn collect_param_names<'a>(params_node: tree_sitter::Node<'a>, source: &'a [u8]) -> Vec<&'a str> {
    let mut names = Vec::new();
    for i in 0..params_node.named_child_count() {
        if let Some(child) = params_node.named_child(i) {
            match child.kind() {
                "identifier" => {
                    let name = node_text(child, source);
                    if name != "self" {
                        names.push(name);
                    }
                }
                "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                    if let Some(name_child) = child.child_by_field_name("name") {
                        let name = node_text(name_child, source);
                        if name != "self" {
                            names.push(name);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    names
}

fn is_path_operation(fn_node: tree_sitter::Node, source: &[u8]) -> (bool, &'static str) {
    match fn_node.kind() {
        "identifier" => {
            let name = node_text(fn_node, source);
            if name == "open" {
                return (true, "unvalidated_path_open");
            }
        }
        "attribute" => {
            let text = node_text(fn_node, source);
            if text.contains("path.join") || text.contains("os.path.join") {
                return (true, "unvalidated_path_join");
            }
        }
        _ => {}
    }
    (false, "")
}

fn args_contain_param(args_node: tree_sitter::Node, source: &[u8], param_names: &[&str]) -> bool {
    let mut stack = vec![args_node];
    while let Some(current) = stack.pop() {
        if current.kind() == "identifier" {
            let name = node_text(current, source);
            if param_names.contains(&name) {
                return true;
            }
        }
        for i in 0..current.named_child_count() {
            if let Some(child) = current.named_child(i) {
                stack.push(child);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_context_file(source, "test.py")
    }

    fn parse_and_check_with_context_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PathTraversalPipeline::new().unwrap();
        let id_counts = HashMap::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: None,
        };
        pipeline.check_with_context(&ctx)
    }

    #[test]
    fn detects_open_with_param() {
        let src = "def read_file(name):\n    return open(\"/base/\" + name)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_open");
    }

    #[test]
    fn ignores_open_with_literal() {
        let src = "def load():\n    return open(\"config.txt\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_no_params() {
        let src = "def load():\n    f = open(filename)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn context_suppresses_validated_path_abspath() {
        let src = r#"def read_file(name):
    safe_path = os.path.abspath(os.path.join("/base", name))
    return open(safe_path)
"#;
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "expected no findings when abspath is called before open, got {:?}",
            findings
        );
    }

    #[test]
    fn context_suppresses_test_file() {
        let src = "def read_file(name):\n    return open(\"/base/\" + name)";
        let findings = parse_and_check_with_context_file(src, "tests/test_utils.py");
        assert!(
            findings.is_empty(),
            "expected no findings in test files, got {:?}",
            findings
        );
    }

    #[test]
    fn context_still_flags_unvalidated_path() {
        let src = r#"def read_file(name):
    return open("/base/" + name)
"#;
        let findings = parse_and_check_with_context(src);
        assert_eq!(
            findings.len(),
            1,
            "expected 1 finding for unvalidated path, got {:?}",
            findings
        );
        assert_eq!(findings[0].pattern, "unvalidated_path_open");
    }

    #[test]
    fn context_suppresses_dotdot_check() {
        let src = r#"def read_file(name):
    if ".." in name:
        raise ValueError("bad path")
    return open("/base/" + name)
"#;
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "expected no findings when '..' check exists before open, got {:?}",
            findings
        );
    }
}
