use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::go_primitives::{
    compile_assignment_query, compile_short_var_decl_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct ErrorSwallowingPipeline {
    short_var_query: Arc<Query>,
    assign_query: Arc<Query>,
}

impl ErrorSwallowingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            short_var_query: compile_short_var_decl_query()?,
            assign_query: compile_assignment_query()?,
        })
    }

    fn check_declaration(
        &self,
        tree: &Tree,
        source: &[u8],
        query: &Query,
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let lhs_idx = find_capture_index(query, "lhs");
        let rhs_idx = find_capture_index(query, "rhs");
        let decl_idx = query
            .capture_names()
            .iter()
            .position(|n| *n == "decl" || *n == "assign")
            .expect("query must have @decl or @assign capture");

        while let Some(m) = matches.next() {
            let lhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == lhs_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == decl_idx)
                .map(|c| c.node);

            if let (Some(lhs), Some(rhs), Some(decl)) = (lhs_node, rhs_node, decl_node) {
                // Check if any LHS element is blank identifier `_`
                let has_blank = (0..lhs.named_child_count()).any(|i| {
                    lhs.named_child(i)
                        .map(|child| child.kind() == "identifier" && node_text(child, source) == "_")
                        .unwrap_or(false)
                });

                if !has_blank {
                    continue;
                }

                // Verify RHS contains a call_expression (not map access, type assertion, etc.)
                let has_call = (0..rhs.named_child_count()).any(|i| {
                    rhs.named_child(i)
                        .map(|child| child.kind() == "call_expression")
                        .unwrap_or(false)
                });

                if !has_call {
                    continue;
                }

                let start = decl.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "error_swallowed".to_string(),
                    message: "error return value discarded with blank identifier `_`".to_string(),
                    snippet: extract_snippet(source, decl, 1),
                });
            }
        }

        findings
    }
}

impl Pipeline for ErrorSwallowingPipeline {
    fn name(&self) -> &str {
        "error_swallowing"
    }

    fn description(&self) -> &str {
        "Detects discarded error returns via blank identifier: `data, _ := someFunc()`"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_declaration(tree, source, &self.short_var_query, file_path));
        findings.extend(self.check_declaration(tree, source, &self.assign_query, file_path));
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ErrorSwallowingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_short_var_decl_error_swallow() {
        let src = "package main\nfunc main() {\n\tdata, _ := someFunc()\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "error_swallowed");
    }

    #[test]
    fn detects_assignment_error_swallow() {
        let src = "package main\nfunc main() {\n\tvar data int\n\tdata, _ = someFunc()\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "error_swallowed");
    }

    #[test]
    fn skips_map_access() {
        let src = "package main\nfunc main() {\n\t_, ok := myMap[key]\n\t_ = ok\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_single_blank_without_call() {
        let src = "package main\nfunc main() {\n\t_ = someValue\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_code_with_error_handling() {
        let src = "package main\nfunc main() {\n\tdata, err := someFunc()\n\tif err != nil {\n\t\treturn\n\t}\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
