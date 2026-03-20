use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_include_require_query, extract_snippet, find_capture_index};

pub struct UnsafeIncludePipeline {
    include_query: Arc<Query>,
}

impl UnsafeIncludePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            include_query: compile_include_require_query()?,
        })
    }
}

impl Pipeline for UnsafeIncludePipeline {
    fn name(&self) -> &str {
        "unsafe_include"
    }

    fn description(&self) -> &str {
        "Detects include/require with dynamic (non-literal) paths"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.include_query, tree.root_node(), source);

        let include_idx = find_capture_index(&self.include_query, "include_expr");

        while let Some(m) = matches.next() {
            let cap = m.captures.iter().find(|c| c.index as usize == include_idx);

            if let Some(cap) = cap {
                let node = cap.node;

                // The argument to include/require is the first named child after the keyword
                // Check if the argument is a static string literal
                let has_dynamic_path = !is_static_include(node);

                if has_dynamic_path {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "dynamic_include".to_string(),
                        message: "dynamic include/require path — use a static string to prevent file inclusion attacks".to_string(),
                        snippet: extract_snippet(source, node, 2),
                    });
                }
            }
        }

        findings
    }
}

fn is_static_include(node: tree_sitter::Node) -> bool {
    // Walk children to find the path argument
    // A static include has only a string literal as argument
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() == "string" {
                return true;
            }
            // If argument is parenthesized_expression, check inside
            if child.kind() == "parenthesized_expression" {
                for j in 0..child.named_child_count() {
                    if let Some(inner) = child.named_child(j)
                        && inner.kind() == "string" {
                            return true;
                        }
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UnsafeIncludePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_variable_include() {
        let src = "<?php\ninclude $file;\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "dynamic_include");
    }

    #[test]
    fn detects_concatenation_require() {
        let src = "<?php\nrequire $dir . '/config.php';\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_static_include() {
        let src = "<?php\ninclude 'config.php';\nrequire_once 'bootstrap.php';\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_dynamic_require_once() {
        let src = "<?php\nrequire_once $base . '/autoload.php';\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
