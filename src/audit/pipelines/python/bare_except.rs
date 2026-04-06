use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_noqa_suppressed;

use super::primitives::{
    compile_except_clause_query, extract_snippet, find_capture_index, node_text,
};

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

impl GraphPipeline for BareExceptPipeline {
    fn name(&self) -> &str {
        "bare_except"
    }

    fn description(&self) -> &str {
        "Detects bare `except:` clauses without specifying an exception type"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.except_query, tree.root_node(), source);

        let except_idx = find_capture_index(&self.except_query, "except");

        while let Some(m) = matches.next() {
            let except_cap = m.captures.iter().find(|c| c.index as usize == except_idx);

            if let Some(except_cap) = except_cap {
                let node = except_cap.node;

                if is_noqa_suppressed(source, node, self.name()) {
                    continue;
                }

                // Separate non-block children: exception type vs bare
                let non_block_children: Vec<_> = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .filter(|child| child.kind() != "block")
                    .collect();

                let has_exception_type = !non_block_children.is_empty();

                if !has_exception_type {
                    // Check if the except block contains a bare `raise` (re-raise pattern)
                    let block = (0..node.named_child_count())
                        .filter_map(|i| node.named_child(i))
                        .find(|child| child.kind() == "block");

                    let has_reraise = block.is_some_and(|block| {
                        (0..block.named_child_count())
                            .filter_map(|i| block.named_child(i))
                            .any(|stmt| {
                                stmt.kind() == "raise_statement"
                                    && stmt.named_child_count() == 0
                            })
                    });

                    if has_reraise {
                        continue; // except: ... raise is a safe cleanup-and-rethrow pattern
                    }

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
                } else {
                    // Check for broad exception types: `except BaseException:`
                    let type_text = non_block_children
                        .first()
                        .map(|n| node_text(*n, source))
                        .unwrap_or("");
                    if type_text == "BaseException" {
                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "broad_exception_handler".to_string(),
                            message: "`except BaseException:` is nearly as broad as bare `except:` — consider catching a more specific exception".to_string(),
                            snippet: extract_snippet(source, node, 2),
                        });
                    }
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
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn skips_bare_except_with_reraise() {
        let src = "try:\n    pass\nexcept:\n    logger.error('failed')\n    raise\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "except: ... raise should be skipped (safe re-raise pattern)"
        );
    }

    #[test]
    fn detects_base_exception() {
        let src = "try:\n    pass\nexcept BaseException:\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "broad_exception_handler");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn noqa_suppresses() {
        let src = "try:\n    pass\nexcept:  # noqa\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "# noqa should suppress");
    }
}
