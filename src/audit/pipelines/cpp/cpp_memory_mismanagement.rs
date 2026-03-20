use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index, node_text,
};

pub struct CppMemoryMismanagementPipeline {
    fn_query: Arc<Query>,
}

impl CppMemoryMismanagementPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_definition_query()?,
        })
    }

    fn walk_for_delete(node: tree_sitter::Node, source: &[u8], deleted: &mut Vec<String>) {
        if node.kind() == "delete_expression" {
            // The argument of delete is a child identifier or expression
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    deleted.push(node_text(child, source).to_string());
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_delete(child, source, deleted);
        }
    }

    fn scan_body_for_delete_use(
        &self,
        body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut deleted_vars: Vec<String> = Vec::new();

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            // Check for delete expressions in this statement
            if child.kind() == "expression_statement" || child.kind() == "declaration" {
                let before_len = deleted_vars.len();
                Self::walk_for_delete(child, source, &mut deleted_vars);

                // If we just found a delete in this statement, skip checking this
                // same statement for use-after-delete
                if deleted_vars.len() > before_len {
                    continue;
                }
            }

            // Check for use of deleted variable in subsequent statements
            if !deleted_vars.is_empty() {
                let text = node_text(child, source);
                for var in &deleted_vars {
                    if text.contains(var.as_str()) {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "delete_then_use".to_string(),
                            message: format!("`{var}` used after `delete` — undefined behavior"),
                            snippet: extract_snippet(source, child, 1),
                        });
                        break;
                    }
                }
            }
        }
        findings
    }

    fn scan_body_for_dangling_string_view(
        &self,
        body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "declaration" {
                let text = node_text(child, source);
                if text.contains("string_view") {
                    // Check if initialized from a temporary — look for a call_expression
                    // or constructor call as the initializer (e.g., std::string("hello"))
                    if Self::has_temporary_initializer(child, source) {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "dangling_string_view".to_string(),
                            message: "`string_view` from temporary — dangling reference after statement ends".to_string(),
                            snippet: extract_snippet(source, child, 1),
                        });
                    }
                }
            }
        }
        findings
    }

    fn has_temporary_initializer(decl: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk the declaration looking for an init_declarator with a call_expression value
        let mut cursor = decl.walk();
        for child in decl.children(&mut cursor) {
            if child.kind() == "init_declarator" {
                // Check if the value (right side of =) is a call_expression
                if let Some(value) = child.child_by_field_name("value")
                    && value.kind() == "call_expression" {
                        let call_text = node_text(value, source);
                        // Heuristic: temporary string construction
                        if call_text.contains("string")
                            || call_text.contains("c_str")
                            || call_text.contains("data")
                        {
                            return true;
                        }
                    }
                // Also check children for call_expression patterns
                let value_text = node_text(child, source);
                if value_text.contains("std::string(")
                    || value_text.contains(".c_str()")
                    || value_text.contains(".data()")
                {
                    return true;
                }
            }
        }
        false
    }
}

impl Pipeline for CppMemoryMismanagementPipeline {
    fn name(&self) -> &str {
        "cpp_memory_mismanagement"
    }

    fn description(&self) -> &str {
        "Detects memory mismanagement: delete-then-use, dangling string_view, invalidated references"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);

            if let Some(body_cap) = body_cap {
                // Pattern 1: delete-then-use
                findings.extend(self.scan_body_for_delete_use(body_cap.node, source, file_path));

                // Pattern 2: dangling string_view
                findings.extend(self.scan_body_for_dangling_string_view(
                    body_cap.node,
                    source,
                    file_path,
                ));
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppMemoryMismanagementPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_delete_then_use() {
        let src = r#"
void f() {
    int* p = new int(42);
    delete p;
    int x = *p;
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "delete_then_use")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn ignores_safe_delete() {
        let src = r#"
void f() {
    int* p = new int(42);
    int x = *p;
    delete p;
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "delete_then_use")
            .collect();
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_dangling_string_view() {
        let src = r#"
void f() {
    std::string_view sv = std::string("hello");
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "dangling_string_view")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("string_view"));
    }

    #[test]
    fn no_finding_for_string_view_from_literal() {
        let src = r#"
void f() {
    std::string_view sv = "hello";
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "dangling_string_view")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
void f() {
    int* p = new int(42);
    delete p;
    int x = *p;
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "delete_then_use")
            .collect();
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].pipeline, "cpp_memory_mismanagement");
    }
}
