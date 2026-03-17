use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_binary_expression_query, compile_function_call_query, extract_snippet,
    find_capture_index, node_text,
};

const SUPERGLOBALS: &[&str] = &["$_GET", "$_POST", "$_REQUEST", "$_COOKIE", "$_SERVER"];

pub struct TypeJugglingPipeline {
    bin_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl TypeJugglingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            bin_query: compile_binary_expression_query()?,
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for TypeJugglingPipeline {
    fn name(&self) -> &str {
        "type_juggling"
    }

    fn description(&self) -> &str {
        "Detects type juggling risks: loose comparison (==) with superglobals, in_array without strict mode"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_loose_comparison(tree, source, file_path, &mut findings);
        self.check_non_strict_in_array(tree, source, file_path, &mut findings);
        findings
    }
}

impl TypeJugglingPipeline {
    fn check_loose_comparison(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.bin_query, tree.root_node(), source);

        let left_idx = find_capture_index(&self.bin_query, "left");
        let right_idx = find_capture_index(&self.bin_query, "right");
        let bin_idx = find_capture_index(&self.bin_query, "bin_expr");

        while let Some(m) = matches.next() {
            let left = m.captures.iter().find(|c| c.index as usize == left_idx).map(|c| c.node);
            let right = m.captures.iter().find(|c| c.index as usize == right_idx).map(|c| c.node);
            let bin_node = m.captures.iter().find(|c| c.index as usize == bin_idx).map(|c| c.node);

            if let (Some(_left), Some(_right), Some(bin_node)) = (left, right, bin_node) {
                let text = node_text(bin_node, source);

                // Check if this is a == (not ===) comparison
                if !text.contains("==") || text.contains("===") {
                    continue;
                }

                // Check if either side involves a superglobal
                let has_superglobal = SUPERGLOBALS.iter().any(|sg| text.contains(sg));
                if !has_superglobal {
                    continue;
                }

                let start = bin_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "loose_comparison".to_string(),
                    message: "loose comparison (==) with superglobal — use strict comparison (===) to prevent type juggling".to_string(),
                    snippet: extract_snippet(source, bin_node, 1),
                });
            }
        }
    }

    fn check_non_strict_in_array(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == fn_name_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(call_node)) = (name_node, args_node, call_node) {
                let fn_name = node_text(name_node, source);
                if fn_name != "in_array" && fn_name != "array_search" {
                    continue;
                }

                // Check if the third argument (strict flag) is present
                // PHP wraps each arg in `argument` nodes
                let arg_count = args_node.named_children(&mut args_node.walk())
                    .filter(|c| c.kind() == "argument")
                    .count();
                if arg_count < 3 {
                    // No strict flag — type juggling risk
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "non_strict_in_array".to_string(),
                        message: format!(
                            "`{fn_name}()` without strict flag — pass `true` as third argument to prevent type juggling"
                        ),
                        snippet: extract_snippet(source, call_node, 1),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypeJugglingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_loose_comparison_get() {
        let src = "<?php\nif ($_GET['pin'] == $stored) { }\n";
        let findings = parse_and_check(src);
        let loose: Vec<_> = findings.iter().filter(|f| f.pattern == "loose_comparison").collect();
        assert_eq!(loose.len(), 1);
    }

    #[test]
    fn ignores_strict_comparison() {
        let src = "<?php\nif ($_GET['pin'] === $stored) { }\n";
        let findings = parse_and_check(src);
        let loose: Vec<_> = findings.iter().filter(|f| f.pattern == "loose_comparison").collect();
        assert!(loose.is_empty());
    }

    #[test]
    fn detects_non_strict_in_array() {
        let src = "<?php\nin_array($role, $allowed);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "non_strict_in_array");
    }

    #[test]
    fn ignores_strict_in_array() {
        let src = "<?php\nin_array($role, $allowed, true);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
