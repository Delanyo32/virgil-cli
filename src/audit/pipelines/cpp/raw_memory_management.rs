use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_delete_expression_query, compile_new_expression_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct RawMemoryManagementPipeline {
    new_query: Arc<Query>,
    delete_query: Arc<Query>,
}

impl RawMemoryManagementPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            new_query: compile_new_expression_query()?,
            delete_query: compile_delete_expression_query()?,
        })
    }

    fn is_smart_ptr_init(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check if the new expression is inside a smart pointer constructor or reset
        if let Some(parent) = node.parent()
            && parent.kind() == "argument_list"
                && let Some(grandparent) = parent.parent() {
                    // Case 1: call_expression — e.g., std::unique_ptr<int>(new int(42))
                    if grandparent.kind() == "call_expression"
                        && let Some(func) = grandparent.child_by_field_name("function") {
                            let func_text = node_text(func, source);
                            if func_text.contains("unique_ptr")
                                || func_text.contains("shared_ptr")
                                || func_text.ends_with("reset")
                            {
                                return true;
                            }
                        }
                    // Case 2: init_declarator — e.g., std::unique_ptr<int> p(new int(42))
                    // The declaration type will contain unique_ptr/shared_ptr
                    if grandparent.kind() == "init_declarator"
                        && let Some(declaration) = grandparent.parent() {
                            let decl_text = node_text(declaration, source);
                            if decl_text.contains("unique_ptr") || decl_text.contains("shared_ptr")
                            {
                                return true;
                            }
                        }
                }
        false
    }
}

impl Pipeline for RawMemoryManagementPipeline {
    fn name(&self) -> &str {
        "raw_memory_management"
    }

    fn description(&self) -> &str {
        "Detects raw new/delete — prefer smart pointers (std::unique_ptr, std::shared_ptr) and std::make_unique/std::make_shared"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Detect raw `new`
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_query, tree.root_node(), source);
            let new_idx = find_capture_index(&self.new_query, "new_expr");

            while let Some(m) = matches.next() {
                let new_cap = m.captures.iter().find(|c| c.index as usize == new_idx);

                if let Some(new_cap) = new_cap {
                    if Self::is_smart_ptr_init(new_cap.node, source) {
                        continue;
                    }

                    let text = node_text(new_cap.node, source);
                    let pattern = if text.contains("[") {
                        "raw_array_allocation"
                    } else {
                        "raw_new_delete"
                    };

                    let start = new_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: "raw `new` — use `std::make_unique` or `std::make_shared` instead".to_string(),
                        snippet: extract_snippet(source, new_cap.node, 1),
                    });
                }
            }
        }

        // Detect raw `delete`
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.delete_query, tree.root_node(), source);
            let delete_idx = find_capture_index(&self.delete_query, "delete_expr");

            while let Some(m) = matches.next() {
                let del_cap = m.captures.iter().find(|c| c.index as usize == delete_idx);

                if let Some(del_cap) = del_cap {
                    let start = del_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "raw_new_delete".to_string(),
                        message: "raw `delete` — smart pointers handle deallocation automatically"
                            .to_string(),
                        snippet: extract_snippet(source, del_cap.node, 1),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawMemoryManagementPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_raw_new() {
        let src = "void f() { int* p = new int(42); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_new_delete");
        assert!(findings[0].message.contains("new"));
    }

    #[test]
    fn detects_raw_delete() {
        let src = "void f(int* p) { delete p; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_new_delete");
        assert!(findings[0].message.contains("delete"));
    }

    #[test]
    fn detects_array_new() {
        let src = "void f() { int* arr = new int[100]; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_array_allocation");
    }

    #[test]
    fn no_finding_for_make_unique() {
        let src = "void f() { auto p = std::make_unique<int>(42); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_smart_ptr_constructor() {
        let src = "void f() { std::unique_ptr<int> p(new int(42)); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_both_new_and_delete() {
        let src = r#"
void f() {
    int* p = new int(10);
    delete p;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn metadata_correct() {
        let src = "void f() { int* p = new int; }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "raw_memory_management");
    }
}
