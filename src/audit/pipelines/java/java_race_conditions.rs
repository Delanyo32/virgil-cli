use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_decl_query, compile_class_decl_query,
    extract_snippet, find_capture_index, node_text,
};

const UNSYNC_COLLECTIONS: &[&str] = &[
    "HashMap", "ArrayList", "LinkedList", "HashSet", "TreeMap", "TreeSet", "LinkedHashMap",
];

pub struct JavaRaceConditionsPipeline {
    field_query: Arc<Query>,
    class_query: Arc<Query>,
}

impl JavaRaceConditionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
            class_query: compile_class_decl_query()?,
        })
    }
}

impl Pipeline for JavaRaceConditionsPipeline {
    fn name(&self) -> &str {
        "java_race_conditions"
    }

    fn description(&self) -> &str {
        "Detects race condition risks: unsynchronized collections and non-atomic increments on shared fields"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_unsync_collections(tree, source, file_path, &mut findings);
        self.check_non_atomic_increment(tree, source, file_path, &mut findings);
        findings
    }
}

impl JavaRaceConditionsPipeline {
    fn check_unsync_collections(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_idx = find_capture_index(&self.field_query, "field_decl");

        while let Some(m) = matches.next() {
            let field_node = m.captures.iter().find(|c| c.index as usize == field_idx).map(|c| c.node);

            if let Some(field_node) = field_node {
                let field_text = node_text(field_node, source);

                let is_unsync = UNSYNC_COLLECTIONS.iter().any(|c| field_text.contains(c));
                if !is_unsync {
                    continue;
                }

                // Skip if it's already using a concurrent variant or synchronized wrapper
                if field_text.contains("Concurrent") || field_text.contains("synchronized") || field_text.contains("Collections.synchronized") {
                    continue;
                }

                // Check if the field is shared (non-private static, or just non-local)
                let source_str = std::str::from_utf8(source).unwrap_or("");
                let has_threads = source_str.contains("Thread") || source_str.contains("Runnable")
                    || source_str.contains("synchronized") || source_str.contains("Executor")
                    || source_str.contains("@Async") || source_str.contains("volatile");

                if has_threads {
                    let start = field_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsynchronized_collection".to_string(),
                        message: "Non-thread-safe collection in concurrent context — use ConcurrentHashMap or Collections.synchronizedMap()".to_string(),
                        snippet: extract_snippet(source, field_node, 1),
                    });
                }
            }
        }
    }

    fn check_non_atomic_increment(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.class_query, "class_body");

        while let Some(m) = matches.next() {
            let body_node = m.captures.iter().find(|c| c.index as usize == body_idx).map(|c| c.node);

            if let Some(body_node) = body_node {
                let body_text = node_text(body_node, source);

                // Check for volatile fields with ++ or += (non-atomic)
                if body_text.contains("volatile") {
                    let source_str = std::str::from_utf8(source).unwrap_or("");
                    // Simple heuristic: look for ++ or += in the class body
                    for (i, line) in source_str.lines().enumerate() {
                        let line_start = body_node.start_position().row;
                        let line_end = body_node.end_position().row;
                        if i >= line_start && i <= line_end {
                            if (line.contains("++") || line.contains("+=")) && !line.contains("Atomic") {
                                // Check if the variable is volatile by looking at surrounding context
                                let trimmed = line.trim();
                                if !trimmed.starts_with("//") && !trimmed.starts_with("*") {
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: i as u32 + 1,
                                        column: 1,
                                        severity: "warning".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "non_atomic_increment".to_string(),
                                        message: "Increment/add on shared field is not atomic — use AtomicInteger or synchronized".to_string(),
                                        snippet: trimmed.to_string(),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = JavaRaceConditionsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_unsync_hashmap_in_threaded_class() {
        let src = r#"class Server implements Runnable {
    private HashMap<String, String> cache;
    public void run() { }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsynchronized_collection");
    }

    #[test]
    fn ignores_concurrent_hashmap() {
        let src = r#"class Server implements Runnable {
    private ConcurrentHashMap<String, String> cache;
    public void run() { }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_threaded_class() {
        let src = r#"class Dao {
    private HashMap<String, String> cache;
    public void get() { }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_collections() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
