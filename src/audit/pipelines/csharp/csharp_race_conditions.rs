use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_class_decl_query, compile_field_decl_query, extract_snippet, find_capture_index,
    node_text,
};

const UNSYNC_COLLECTIONS: &[&str] = &[
    "Dictionary",
    "List",
    "HashSet",
    "Queue",
    "Stack",
    "LinkedList",
    "SortedDictionary",
    "SortedList",
    "SortedSet",
];

pub struct CSharpRaceConditionsPipeline {
    field_query: Arc<Query>,
    class_query: Arc<Query>,
}

impl CSharpRaceConditionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
            class_query: compile_class_decl_query()?,
        })
    }
}

impl Pipeline for CSharpRaceConditionsPipeline {
    fn name(&self) -> &str {
        "csharp_race_conditions"
    }

    fn description(&self) -> &str {
        "Detects race condition risks: unsynchronized collections, non-atomic increments, TOCTOU file checks"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_unsync_collections(tree, source, file_path, &mut findings);
        self.check_non_atomic(tree, source, file_path, &mut findings);
        self.check_toctou(tree, source, file_path, &mut findings);
        findings
    }
}

impl CSharpRaceConditionsPipeline {
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

        let source_str = std::str::from_utf8(source).unwrap_or("");
        let has_threads = source_str.contains("Task")
            || source_str.contains("Thread")
            || source_str.contains("async")
            || source_str.contains("lock")
            || source_str.contains("Parallel")
            || source_str.contains("volatile");

        if !has_threads {
            return;
        }

        while let Some(m) = matches.next() {
            let field_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_idx)
                .map(|c| c.node);

            if let Some(field_node) = field_node {
                let field_text = node_text(field_node, source);

                let is_unsync = UNSYNC_COLLECTIONS.iter().any(|c| field_text.contains(c));
                if !is_unsync {
                    continue;
                }

                // Skip if already concurrent or locked
                if field_text.contains("Concurrent") || field_text.contains("Immutable") {
                    continue;
                }

                let start = field_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unsynchronized_collection".to_string(),
                    message: "Non-thread-safe collection in concurrent context — use ConcurrentDictionary or lock".to_string(),
                    snippet: extract_snippet(source, field_node, 1),
                });
            }
        }
    }

    fn check_non_atomic(
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
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body_node) = body_node {
                let body_text = node_text(body_node, source);

                // Check for volatile/shared fields with ++ or +=
                if body_text.contains("volatile") || body_text.contains("static") {
                    let source_str = std::str::from_utf8(source).unwrap_or("");
                    let has_threads = source_str.contains("Task")
                        || source_str.contains("Thread")
                        || source_str.contains("async")
                        || source_str.contains("Parallel");

                    if has_threads {
                        for (i, line) in source_str.lines().enumerate() {
                            let line_start = body_node.start_position().row;
                            let line_end = body_node.end_position().row;
                            if i >= line_start && i <= line_end
                                && (line.contains("++") || line.contains("+="))
                                    && !line.contains("Interlocked")
                                {
                                    let trimmed = line.trim();
                                    if !trimmed.starts_with("//") && !trimmed.starts_with("*") {
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: i as u32 + 1,
                                            column: 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "non_atomic_increment".to_string(),
                                            message: "Increment on shared field is not atomic — use Interlocked.Increment()".to_string(),
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

    fn check_toctou(
        &self,
        _tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let source_str = std::str::from_utf8(source).unwrap_or("");

        // Look for File.Exists followed by file operations (TOCTOU pattern)
        if source_str.contains("File.Exists") {
            for (i, line) in source_str.lines().enumerate() {
                if line.contains("File.Exists") {
                    // Check subsequent lines for file operations
                    let remaining: String = source_str
                        .lines()
                        .skip(i + 1)
                        .take(5)
                        .collect::<Vec<_>>()
                        .join("\n");
                    let has_file_op = remaining.contains("File.Read")
                        || remaining.contains("File.Write")
                        || remaining.contains("File.Open")
                        || remaining.contains("File.Delete");

                    if has_file_op {
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: i as u32 + 1,
                            column: 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "toctou_file_check".to_string(),
                            message: "File.Exists check followed by file operation — TOCTOU race condition; use try/catch instead".to_string(),
                            snippet: line.trim().to_string(),
                        });
                        break;
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CSharpRaceConditionsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_unsync_dictionary_in_async_class() {
        let src = r#"class Server {
    private Dictionary<string, string> _cache;
    async Task Handle() { }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsynchronized_collection");
    }

    #[test]
    fn detects_toctou() {
        let src = r#"class Foo {
    void Process(string path) {
        if (File.Exists(path)) {
            File.ReadAllText(path);
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "toctou_file_check");
    }

    #[test]
    fn ignores_concurrent_dictionary() {
        let src = r#"class Server {
    private ConcurrentDictionary<string, string> _cache;
    async Task Handle() { }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_collections() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
