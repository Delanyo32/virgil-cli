use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_decl_query, compile_param_decl_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct NakedInterfacePipeline {
    param_query: Arc<Query>,
    field_query: Arc<Query>,
}

impl NakedInterfacePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_param_decl_query()?,
            field_query: compile_field_decl_query()?,
        })
    }

    fn is_naked_interface(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check for `interface{}` — an interface_type with no method specs
        if node.kind() == "interface_type" {
            // Empty interface has no named children (no method specs)
            return node.named_child_count() == 0;
        }
        // Check for `any` type identifier
        if node.kind() == "type_identifier" && node_text(node, source) == "any" {
            return true;
        }
        false
    }

    fn check_with_query(
        &self,
        tree: &Tree,
        source: &[u8],
        query: &Query,
        type_capture: &str,
        decl_capture: &str,
        file_path: &str,
        pattern: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let type_idx = find_capture_index(query, type_capture);
        let decl_idx = find_capture_index(query, decl_capture);

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == decl_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(decl_node)) = (type_node, decl_node) {
                if Self::is_naked_interface(type_node, source) {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: "empty interface (`interface{}` / `any`) loses type safety — consider a concrete interface".to_string(),
                        snippet: extract_snippet(source, decl_node, 1),
                    });
                }
            }
        }

        findings
    }
}

impl Pipeline for NakedInterfacePipeline {
    fn name(&self) -> &str {
        "naked_interface"
    }

    fn description(&self) -> &str {
        "Detects use of `interface{}` or `any` as parameter or field types"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.param_query,
            "param_type",
            "param",
            file_path,
            "empty_interface_param",
        ));
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.field_query,
            "field_type",
            "field",
            file_path,
            "empty_interface_field",
        ));
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
        let pipeline = NakedInterfacePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_empty_interface_param() {
        let src = "package main\nfunc Process(items interface{}) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_interface_param");
    }

    #[test]
    fn detects_any_param() {
        let src = "package main\nfunc Process(items any) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_interface_param");
    }

    #[test]
    fn clean_concrete_interface() {
        let src = "package main\ntype Stringer interface { String() string }\nfunc Process(items Stringer) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_empty_interface_field() {
        let src = "package main\ntype Config struct {\n\tValue interface{}\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
