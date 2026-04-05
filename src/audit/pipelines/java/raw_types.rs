use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::has_suppress_warnings;

use super::primitives::{
    compile_raw_type_field_query, compile_raw_type_local_query, compile_raw_type_param_query,
    extract_snippet, find_capture_index, has_modifier, node_text,
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
    "Supplier",
    "Function",
    "Consumer",
    "Predicate",
    "BiFunction",
    "Callable",
    "ConcurrentHashMap",
    "BlockingQueue",
    "ThreadLocal",
    "WeakReference",
    "SoftReference",
    "AtomicReference",
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
        tree: &tree_sitter::Tree,
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
                    // Skip if @SuppressWarnings("rawtypes") is present
                    if has_suppress_warnings(type_node, source, "rawtypes") {
                        continue;
                    }
                    let var_name = node_text(name_node, source);
                    let start = type_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: self.severity_for_context(type_node, source),
                        pipeline: self.name().to_string(),
                        pattern: "raw_generic_type".to_string(),
                        message: format!(
                            "`{type_text} {var_name}` uses a raw type — add type parameters (e.g. `{type_text}<String>`)"
                        ),
                        snippet: extract_snippet(
                            source,
                            type_node.parent().unwrap_or(type_node),
                            3,
                        ),
                    });
                }
            }
        }

        findings
    }

    fn check_return_types(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.walk_for_raw_return_types(tree.root_node(), source, file_path, &mut findings);
        findings
    }

    fn walk_for_raw_return_types(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        if node.kind() == "method_declaration"
            && let Some(type_node) = node.child_by_field_name("type")
            && type_node.kind() == "type_identifier"
        {
            let type_text = node_text(type_node, source);
            if self.known_generics.contains(type_text) {
                // Skip if @SuppressWarnings("rawtypes") is present
                if !has_suppress_warnings(type_node, source, "rawtypes") {
                    let method_name = node
                        .child_by_field_name("name")
                        .map(|n| node_text(n, source))
                        .unwrap_or("unknown");
                    let start = type_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: self.severity_for_context(node, source),
                        pipeline: self.name().to_string(),
                        pattern: "raw_generic_type".to_string(),
                        message: format!(
                            "method `{method_name}` returns raw `{type_text}` — add type parameters"
                        ),
                        snippet: extract_snippet(source, node, 3),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_for_raw_return_types(child, source, file_path, findings);
        }
    }

    fn check_cast_expressions(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        if node.kind() == "cast_expression"
            && let Some(type_node) = node.child_by_field_name("type")
            && type_node.kind() == "type_identifier"
        {
            let type_text = node_text(type_node, source);
            if self.known_generics.contains(type_text) {
                // Skip if @SuppressWarnings("rawtypes") is present
                if !has_suppress_warnings(type_node, source, "rawtypes") {
                    let start = type_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "raw_generic_type".to_string(),
                        message: format!(
                            "cast to raw `{type_text}` — add type parameters"
                        ),
                        snippet: extract_snippet(source, node, 3),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.check_cast_expressions(child, source, file_path, findings);
        }
    }

    fn severity_for_context(&self, node: tree_sitter::Node, source: &[u8]) -> String {
        // Check if the raw type is in a public API context
        let mut parent = Some(node);
        while let Some(p) = parent {
            if p.kind() == "method_declaration" || p.kind() == "field_declaration" {
                if has_modifier(p, source, "public") {
                    return "error".to_string();
                }
                return "warning".to_string();
            }
            parent = p.parent();
        }
        "warning".to_string()
    }
}

impl GraphPipeline for RawTypesPipeline {
    fn name(&self) -> &str {
        "raw_types"
    }

    fn description(&self) -> &str {
        "Detects raw generic types (e.g. List instead of List<String>) — add type parameters"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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
        findings.extend(self.check_return_types(tree, source, file_path));
        {
            let mut cast_findings = Vec::new();
            self.check_cast_expressions(
                tree.root_node(),
                source,
                file_path,
                &mut cast_findings,
            );
            findings.extend(cast_findings);
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawTypesPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Test.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn test_raw_return_type() {
        let src = "class Foo { List getData() { return null; } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("returns raw"));
    }

    #[test]
    fn test_raw_cast() {
        let src = "class Foo { void m(Object obj) { List items = (List) obj; } }";
        let findings = parse_and_check(src);
        // Should detect both the raw local and the raw cast
        assert!(findings.len() >= 1);
        assert!(findings.iter().any(|f| f.message.contains("cast")));
    }

    #[test]
    fn test_suppress_rawtypes() {
        let src = r#"class Foo { @SuppressWarnings("rawtypes") List items; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_public_vs_private_severity() {
        let src = r#"
class Foo {
    public List publicItems;
    private List privateItems;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let pub_f = findings
            .iter()
            .find(|f| f.message.contains("publicItems"))
            .unwrap();
        let priv_f = findings
            .iter()
            .find(|f| f.message.contains("privateItems"))
            .unwrap();
        assert_eq!(pub_f.severity, "error");
        assert_eq!(priv_f.severity, "warning");
    }
}
