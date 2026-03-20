use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_comparison_query, extract_snippet, find_capture_index, node_text};

const SUSPICIOUS_NAMES: &[&str] = &[
    "status", "kind", "type", "mode", "state", "action", "level", "category", "role", "variant",
    "phase", "stage",
];

pub struct StringlyTypedPipeline {
    comparison_query: Arc<Query>,
}

impl StringlyTypedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            comparison_query: compile_comparison_query()?,
        })
    }

    fn is_suspicious_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        SUSPICIOUS_NAMES.iter().any(|s| lower.contains(s))
    }
}

impl Pipeline for StringlyTypedPipeline {
    fn name(&self) -> &str {
        "stringly_typed"
    }

    fn description(&self) -> &str {
        "Detects string comparisons on field names that should be enums"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.comparison_query, tree.root_node(), source);

        let comp_idx = find_capture_index(&self.comparison_query, "comparison");

        while let Some(m) = matches.next() {
            let comp_cap = m.captures.iter().find(|c| c.index as usize == comp_idx);

            if let Some(comp_cap) = comp_cap {
                let node = comp_cap.node;

                // Look for a string literal and an identifier/attribute among children
                let mut has_string = false;
                let mut suspicious_identifier = None;

                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        match child.kind() {
                            "string" => {
                                has_string = true;
                            }
                            "identifier" => {
                                let name = node_text(child, source);
                                if Self::is_suspicious_name(name) {
                                    suspicious_identifier = Some(name.to_string());
                                }
                            }
                            "attribute" => {
                                // For obj.status, get the attribute name
                                if let Some(attr) = child.child_by_field_name("attribute") {
                                    let name = node_text(attr, source);
                                    if Self::is_suspicious_name(name) {
                                        suspicious_identifier =
                                            Some(node_text(child, source).to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if has_string
                    && let Some(ident) = suspicious_identifier {
                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "stringly_typed_comparison".to_string(),
                            message: format!(
                                "string comparison on `{ident}` — consider using an enum instead"
                            ),
                            snippet: extract_snippet(source, node, 1),
                        });
                    }
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StringlyTypedPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_status_string_comparison() {
        let src = "if status == \"active\":\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "stringly_typed_comparison");
    }

    #[test]
    fn detects_attribute_comparison() {
        let src = "if obj.state == \"running\":\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_numeric_comparison() {
        let src = "if x == 5:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_suspicious_name() {
        let src = "if name == \"alice\":\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
