use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_declaration_query, extract_snippet, find_capture_index, find_identifier_in_declarator,
    has_storage_class, has_type_qualifier,
};

pub struct GlobalMutableStatePipeline {
    decl_query: Arc<Query>,
}

impl GlobalMutableStatePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            decl_query: compile_declaration_query()?,
        })
    }

    fn is_function_declarator(node: tree_sitter::Node) -> bool {
        if node.kind() == "function_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::is_function_declarator(inner);
        }
        false
    }
}

impl Pipeline for GlobalMutableStatePipeline {
    fn name(&self) -> &str {
        "global_mutable_state"
    }

    fn description(&self) -> &str {
        "Detects file-scope non-const mutable variables that create hidden shared state"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.decl_query, tree.root_node(), source);

        let decl_idx = find_capture_index(&self.decl_query, "decl");

        while let Some(m) = matches.next() {
            let decl_cap = m.captures.iter().find(|c| c.index as usize == decl_idx);

            if let Some(decl_cap) = decl_cap {
                let decl_node = decl_cap.node;

                // Must be at translation_unit scope (file-level)
                if let Some(parent) = decl_node.parent() {
                    if parent.kind() != "translation_unit" {
                        continue;
                    }
                } else {
                    continue;
                }

                // Skip const declarations
                if has_type_qualifier(decl_node, source, "const") {
                    continue;
                }

                // Skip extern declarations
                if has_storage_class(decl_node, source, "extern") {
                    continue;
                }

                // Skip function prototypes
                if let Some(declarator) = decl_node.child_by_field_name("declarator")
                    && Self::is_function_declarator(declarator)
                {
                    continue;
                }

                let var_name = decl_node
                    .child_by_field_name("declarator")
                    .and_then(|d| find_identifier_in_declarator(d, source))
                    .unwrap_or_default();

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "global_mutable_state".to_string(),
                    message: format!(
                        "global mutable variable `{var_name}` — consider encapsulating or making const"
                    ),
                    snippet: extract_snippet(source, decl_node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GlobalMutableStatePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_global_mutable() {
        let src = "int global_count = 0;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_state");
        assert!(findings[0].message.contains("global_count"));
    }

    #[test]
    fn skips_const() {
        let src = "const int MAX = 100;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_extern() {
        let src = "extern int shared_count;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_function_prototype() {
        let src = "int main(int argc, char **argv);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_local_variable() {
        let src = "void f() { int local = 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
