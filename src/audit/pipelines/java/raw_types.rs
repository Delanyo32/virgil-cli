use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_raw_type_field_query, compile_raw_type_local_query, compile_raw_type_param_query,
    extract_snippet, find_capture_index, node_text,
};

const KNOWN_GENERICS: &[&str] = &[
    "List",
    "Map",
    "Set",
    "Collection",
    "ArrayList",
    "HashMap",
    "HashSet",
    "LinkedList",
    "TreeMap",
    "TreeSet",
    "Queue",
    "Deque",
    "Stack",
    "Vector",
    "Iterator",
    "Iterable",
    "Optional",
    "Stream",
    "Future",
    "CompletableFuture",
    "Class",
    "Comparable",
];

pub struct RawTypesPipeline {
    field_query: Arc<Query>,
    local_query: Arc<Query>,
    param_query: Arc<Query>,
    known_generics: HashSet<&'static str>,
}

impl RawTypesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_raw_type_field_query()?,
            local_query: compile_raw_type_local_query()?,
            param_query: compile_raw_type_param_query()?,
            known_generics: KNOWN_GENERICS.iter().copied().collect(),
        })
    }

    fn check_query(
        &self,
        query: &Query,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        type_capture: &str,
        name_capture: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let type_idx = find_capture_index(query, type_capture);
        let name_idx = find_capture_index(query, name_capture);

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(name_node)) = (type_node, name_node) {
                let type_text = node_text(type_node, source);
                if self.known_generics.contains(type_text) {
                    let var_name = node_text(name_node, source);
                    let start = type_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "raw_generic_type".to_string(),
                        message: format!(
                            "`{type_text} {var_name}` uses a raw type — add type parameters (e.g. `{type_text}<String>`)"
                        ),
                        snippet: extract_snippet(source, type_node.parent().unwrap_or(type_node), 3),
                    });
                }
            }
        }

        findings
    }
}

impl Pipeline for RawTypesPipeline {
    fn name(&self) -> &str {
        "raw_types"
    }

    fn description(&self) -> &str {
        "Detects raw generic types (e.g. List instead of List<String>) — add type parameters"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_query(
            &self.field_query,
            tree,
            source,
            file_path,
            "raw_type",
            "var_name",
        ));
        findings.extend(self.check_query(
            &self.local_query,
            tree,
            source,
            file_path,
            "raw_type",
            "var_name",
        ));
        findings.extend(self.check_query(
            &self.param_query,
            tree,
            source,
            file_path,
            "raw_type",
            "param_name",
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawTypesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_raw_field() {
        let src = "class Foo { List items; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_generic_type");
        assert!(findings[0].message.contains("List"));
    }

    #[test]
    fn detects_raw_local() {
        let src = "class Foo { void m() { Map data = null; } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("Map"));
    }

    #[test]
    fn detects_raw_param() {
        let src = "class Foo { void m(Set items) { } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("Set"));
    }

    #[test]
    fn clean_parameterized_type() {
        let src = "class Foo { List<String> items; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_generic_type() {
        let src = "class Foo { String name; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_primitive() {
        let src = "class Foo { int x = 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
