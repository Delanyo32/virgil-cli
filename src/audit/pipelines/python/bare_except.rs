use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_except_clause_query, extract_snippet, find_capture_index};

pub struct BareExceptPipeline {
    except_query: Arc<Query>,
}

impl BareExceptPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            except_query: compile_except_clause_query()?,
        })
    }
}

impl Pipeline for BareExceptPipeline {
    fn name(&self) -> &str {
        "bare_except"
    }

    fn description(&self) -> &str {
        "Detects bare `except:` clauses without specifying an exception type"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.except_query, tree.root_node(), source);

        let except_idx = find_capture_index(&self.except_query, "except");

        while let Some(m) = matches.next() {
            let except_cap = m.captures.iter().find(|c| c.index as usize == except_idx);

            if let Some(except_cap) = except_cap {
                let node = except_cap.node;

                // A bare except has no named children before the block body.
                // except_clause children: optional exception type, optional "as" name, then block.
                // Count named children that are NOT a block — if 0, it's bare.
                let has_exception_type = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .any(|child| child.kind() != "block");

                if !has_exception_type {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "untyped_exception_handler".to_string(),
                        message: "bare `except:` catches all exceptions including SystemExit and KeyboardInterrupt — specify an exception type".to_string(),
                        snippet: extract_snippet(source, node, 2),
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
        let pipeline = BareExceptPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_bare_except() {
        let src = "try:\n    pass\nexcept:\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "untyped_exception_handler");
    }

    #[test]
    fn skips_typed_except() {
        let src = "try:\n    pass\nexcept Exception:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_except_as() {
        let src = "try:\n    pass\nexcept Exception as e:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_tuple_except() {
        let src = "try:\n    pass\nexcept (ValueError, TypeError):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
