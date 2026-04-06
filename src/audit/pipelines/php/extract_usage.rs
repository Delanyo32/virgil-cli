use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

/// PHP superglobals -- extracting these into scope is a security risk.
const SUPERGLOBALS: &[&str] = &[
    "$_POST",
    "$_GET",
    "$_REQUEST",
    "$_COOKIE",
    "$_SERVER",
    "$_FILES",
    "$_ENV",
];

/// Extract flags that make extract() safer.
const SAFE_FLAGS: &[&str] = &[
    "EXTR_IF_EXISTS",
    "EXTR_SKIP",
    "EXTR_PREFIX_ALL",
    "EXTR_PREFIX_IF_EXISTS",
];

pub struct ExtractUsagePipeline {
    call_query: Arc<Query>,
}

impl ExtractUsagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl NodePipeline for ExtractUsagePipeline {
    fn name(&self) -> &str {
        "extract_usage"
    }

    fn description(&self) -> &str {
        "Detects use of extract() which pollutes the local scope with untracked variables"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
                let fn_name = node_text(name_node, source);
                if fn_name != "extract" {
                    continue;
                }

                if is_nolint_suppressed(source, call_node, self.name()) {
                    continue;
                }

                // Get the arguments node
                let args_node = call_node.child_by_field_name("arguments");

                // Check first argument for superglobals
                let first_arg_text = args_node
                    .and_then(|args| args.named_child(0))
                    .map(|arg| node_text(arg, source));

                let is_superglobal = first_arg_text
                    .map(|text| SUPERGLOBALS.contains(&text))
                    .unwrap_or(false);

                // Check second argument for safe flags
                let has_safe_flag = args_node
                    .and_then(|args| args.named_child(1))
                    .map(|arg| {
                        let text = node_text(arg, source);
                        SAFE_FLAGS.contains(&text)
                    })
                    .unwrap_or(false);

                let (severity, pattern, message) = if is_superglobal {
                    (
                        "error",
                        "extract_superglobal",
                        format!(
                            "extract({}) injects user input directly into scope — critical security risk",
                            first_arg_text.unwrap_or("?")
                        ),
                    )
                } else if has_safe_flag {
                    (
                        "info",
                        "extract_call",
                        "extract() with safe flag reduces risk — still prefer explicit array access".to_string(),
                    )
                } else {
                    (
                        "warning",
                        "extract_call",
                        "extract() pollutes the local scope with untracked variables — use explicit array access instead".to_string(),
                    )
                };

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message,
                    snippet: extract_snippet(source, call_node, 2),
                });
            }
        }

        findings
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
        let pipeline = ExtractUsagePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_extract() {
        let src = "<?php\nextract($_POST);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_no_extract() {
        let src = "<?php\n$name = $_POST['name'];\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_other_functions() {
        let src = "<?php\ncompact('a', 'b');\narray_merge($a, $b);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    // --- New tests ---

    #[test]
    fn extract_superglobal_error() {
        let src = "<?php\nextract($_POST);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].pattern, "extract_superglobal");
    }

    #[test]
    fn extract_get_superglobal_error() {
        let src = "<?php\nextract($_GET);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn extract_local_array_warning() {
        let src = "<?php\n$data = ['x' => 1];\nextract($data);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn extract_with_safe_flag_info() {
        let src = "<?php\nextract($data, EXTR_IF_EXISTS);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn extract_with_skip_flag_info() {
        let src = "<?php\nextract($data, EXTR_SKIP);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "<?php\n// NOLINT(extract_usage)\nextract($_POST);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
