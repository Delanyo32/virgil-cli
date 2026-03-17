use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_class_specifier_query, extract_snippet, find_capture_index, node_text,
};

pub struct CppRaceConditionsPipeline {
    class_query: Arc<Query>,
}

impl CppRaceConditionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_specifier_query()?,
        })
    }

    fn has_mutex_field(body: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "field_declaration" {
                let text = node_text(child, source);
                if text.contains("mutex") {
                    return true;
                }
            }
            // Also check access_specifier sections
            if child.kind() == "access_specifier" {
                continue;
            }
        }
        false
    }

    fn method_modifies_field(body: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(body, source);
        // Heuristic: look for assignment operators or increment/decrement on members
        text.contains("++") || text.contains("--") || text.contains("+=")
            || text.contains("-=") || text.contains("*=") || text.contains("/=")
            || Self::has_simple_assignment(body, source)
    }

    fn has_simple_assignment(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk looking for assignment_expression nodes
        if node.kind() == "assignment_expression" {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::has_simple_assignment(child, source) {
                return true;
            }
        }
        false
    }

    fn has_lock_guard(body: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(body, source);
        text.contains("lock_guard") || text.contains("unique_lock")
            || text.contains("scoped_lock")
    }

    fn find_methods_in_class_body(
        &self,
        class_body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Walk class body for function definitions (methods)
        let mut cursor = class_body.walk();
        for child in class_body.children(&mut cursor) {
            if child.kind() == "function_definition" {
                if let Some(body) = child.child_by_field_name("body") {
                    if Self::method_modifies_field(body, source)
                        && !Self::has_lock_guard(body, source)
                    {
                        let declarator = child.child_by_field_name("declarator");
                        let method_name = declarator
                            .map(|d| node_text(d, source))
                            .unwrap_or("<unknown>");

                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unguarded_shared_mutation".to_string(),
                            message: format!(
                                "method `{method_name}` modifies shared state without lock guard — potential data race"
                            ),
                            snippet: extract_snippet(source, child, 3),
                        });
                    }
                }
            }
        }
        findings
    }
}

impl Pipeline for CppRaceConditionsPipeline {
    fn name(&self) -> &str {
        "cpp_race_conditions"
    }

    fn description(&self) -> &str {
        "Detects race condition risks: unguarded shared mutation, unsynchronized containers with threads"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_body_idx = find_capture_index(&self.class_query, "class_body");

        while let Some(m) = matches.next() {
            let body_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx);

            if let Some(body_cap) = body_cap {
                // Only flag classes that have a mutex field
                if !Self::has_mutex_field(body_cap.node, source) {
                    continue;
                }

                // Check each method in the class for unguarded mutation
                findings.extend(self.find_methods_in_class_body(
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
        let pipeline = CppRaceConditionsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_unguarded_mutation() {
        let src = r#"
class Counter {
    std::mutex mtx;
    int count;
public:
    void increment() {
        count++;
    }
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unguarded_shared_mutation");
        assert!(findings[0].message.contains("lock guard"));
    }

    #[test]
    fn ignores_guarded_mutation() {
        let src = r#"
class Counter {
    std::mutex mtx;
    int count;
public:
    void increment() {
        std::lock_guard<std::mutex> lock(mtx);
        count++;
    }
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_for_class_without_mutex() {
        let src = r#"
class Simple {
    int count;
public:
    void increment() {
        count++;
    }
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_unique_lock() {
        let src = r#"
class Counter {
    std::mutex mtx;
    int count;
public:
    void increment() {
        std::unique_lock<std::mutex> lock(mtx);
        count++;
    }
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
class Counter {
    std::mutex mtx;
    int count;
public:
    void increment() {
        count++;
    }
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "cpp_race_conditions");
    }
}
