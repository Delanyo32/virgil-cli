use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const DISPOSABLE_TYPES: &[&str] = &[
    "SqlConnection",
    "HttpClient",
    "StreamReader",
    "StreamWriter",
    "FileStream",
    "MemoryStream",
    "TcpClient",
    "WebClient",
    "SqlCommand",
    "SqlDataReader",
    "NetworkStream",
    "BinaryReader",
    "BinaryWriter",
    "CryptoStream",
];

const COLLECTION_ADD_METHODS: &[&str] = &["Add", "Insert", "Enqueue", "Push"];

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    object_creation_query: Arc<Query>,
    using_statement_query: Arc<Query>,
    assignment_query: Arc<Query>,
    loop_query: Arc<Query>,
    invocation_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let object_creation_query_str = r#"
(object_creation_expression
  type: (_) @type_name) @creation
"#;
        let object_creation_query = Query::new(&csharp_lang(), object_creation_query_str)
            .with_context(|| {
                "failed to compile object_creation query for memory_leak_indicators"
            })?;

        let using_statement_query_str = r#"
(using_statement) @using_stmt
"#;
        let using_statement_query = Query::new(&csharp_lang(), using_statement_query_str)
            .with_context(|| {
                "failed to compile using_statement query for memory_leak_indicators"
            })?;

        let assignment_query_str = r#"
(assignment_expression
  left: (_) @lhs
  right: (_) @rhs) @assign
"#;
        let assignment_query = Query::new(&csharp_lang(), assignment_query_str)
            .with_context(|| {
                "failed to compile assignment query for memory_leak_indicators"
            })?;

        let loop_query_str = r#"
[
  (for_statement
    body: (_) @loop_body) @loop
  (foreach_statement
    body: (_) @loop_body) @loop
  (while_statement
    body: (_) @loop_body) @loop
  (do_statement
    body: (_) @loop_body) @loop
]
"#;
        let loop_query = Query::new(&csharp_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for memory_leak_indicators")?;

        let invocation_query_str = r#"
(invocation_expression
  function: (_) @fn_expr
  arguments: (argument_list) @args) @invocation
"#;
        let invocation_query = Query::new(&csharp_lang(), invocation_query_str)
            .with_context(|| "failed to compile invocation query for memory_leak_indicators")?;

        Ok(Self {
            object_creation_query: Arc::new(object_creation_query),
            using_statement_query: Arc::new(using_statement_query),
            assignment_query: Arc::new(assignment_query),
            loop_query: Arc::new(loop_query),
            invocation_query: Arc::new(invocation_query),
        })
    }

    fn collect_using_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<std::ops::Range<usize>> {
        let mut ranges = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.using_statement_query, tree.root_node(), source);

        let using_idx = find_capture_index(&self.using_statement_query, "using_stmt");

        while let Some(m) = matches.next() {
            if let Some(cap) = m
                .captures
                .iter()
                .find(|c| c.index as usize == using_idx)
            {
                ranges.push(cap.node.start_byte()..cap.node.end_byte());
            }
        }

        ranges
    }

    fn is_inside_using(using_ranges: &[std::ops::Range<usize>], byte_offset: usize) -> bool {
        using_ranges.iter().any(|r| r.contains(&byte_offset))
    }

    fn check_disposable_without_using(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        using_ranges: &[std::ops::Range<usize>],
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.object_creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.object_creation_query, "type_name");
        let creation_idx = find_capture_index(&self.object_creation_query, "creation");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(creation_node)) = (type_node, creation_node) {
                let type_text = node_text(type_node, source);
                if DISPOSABLE_TYPES.contains(&type_text)
                    && !Self::is_inside_using(using_ranges, creation_node.start_byte())
                {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "disposable_without_using".to_string(),
                        message: format!(
                            "`new {type_text}(...)` created without `using` statement \u{2014} may leak unmanaged resources"
                        ),
                        snippet: extract_snippet(source, creation_node, 1),
                    });
                }
            }
        }

        findings
    }

    fn check_event_handler_leaks(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assignment_query, tree.root_node(), source);

        let assign_idx = find_capture_index(&self.assignment_query, "assign");
        let lhs_idx = find_capture_index(&self.assignment_query, "lhs");

        // Track += and -= on same names
        let mut subscriptions: Vec<(String, tree_sitter::Node)> = Vec::new();
        let mut unsubscriptions: std::collections::HashSet<String> = std::collections::HashSet::new();

        while let Some(m) = matches.next() {
            let assign_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assign_idx)
                .map(|c| c.node);
            let lhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == lhs_idx)
                .map(|c| c.node);

            if let (Some(assign_node), Some(lhs_node)) = (assign_node, lhs_node) {
                let assign_text = node_text(assign_node, source);
                let lhs_text = node_text(lhs_node, source);

                if assign_text.contains("+=") {
                    subscriptions.push((lhs_text.to_string(), assign_node));
                } else if assign_text.contains("-=") {
                    unsubscriptions.insert(lhs_text.to_string());
                }
            }
        }

        for (event_name, assign_node) in &subscriptions {
            if !unsubscriptions.contains(event_name) {
                let start = assign_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "event_handler_leak".to_string(),
                    message: format!(
                        "event subscription `{event_name} +=` without corresponding `-=` unsubscription \u{2014} may cause memory leaks"
                    ),
                    snippet: extract_snippet(source, *assign_node, 1),
                });
            }
        }

        findings
    }

    fn check_unbounded_collection_growth(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.loop_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.loop_query, "loop_body");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let mut inv_cursor = QueryCursor::new();
                let mut inv_matches =
                    inv_cursor.matches(&self.invocation_query, body, source);

                let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
                let inv_idx = find_capture_index(&self.invocation_query, "invocation");

                while let Some(im) = inv_matches.next() {
                    let fn_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == fn_idx)
                        .map(|c| c.node);
                    let inv_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == inv_idx)
                        .map(|c| c.node);

                    if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                        let fn_text = node_text(fn_node, source);
                        for &method in COLLECTION_ADD_METHODS {
                            if fn_text.ends_with(method) {
                                let start = inv_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unbounded_collection_growth".to_string(),
                                    message: format!(
                                        "`.{method}()` inside loop \u{2014} collection may grow unboundedly and cause memory pressure"
                                    ),
                                    snippet: extract_snippet(source, inv_node, 1),
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }

        findings
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory and resource leaks (missing using, event handler leaks, unbounded collections)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let using_ranges = self.collect_using_ranges(tree, source);
        findings.extend(self.check_disposable_without_using(tree, source, file_path, &using_ranges));
        findings.extend(self.check_event_handler_leaks(tree, source, file_path));
        findings.extend(self.check_unbounded_collection_growth(tree, source, file_path));

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_sql_connection_without_using() {
        let src = r#"
class DataAccess {
    void Query() {
        var conn = new SqlConnection("connstring");
        conn.Open();
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "disposable_without_using")
            .collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].message.contains("SqlConnection"));
    }

    #[test]
    fn detects_http_client_without_using() {
        let src = r#"
class Fetcher {
    void Fetch() {
        var client = new HttpClient();
        client.GetAsync("http://example.com");
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "disposable_without_using")
            .collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].message.contains("HttpClient"));
    }

    #[test]
    fn ignores_disposable_inside_using() {
        let src = r#"
class DataAccess {
    void Query() {
        using (var conn = new SqlConnection("connstring")) {
            conn.Open();
        }
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "disposable_without_using")
            .collect();
        assert!(matched.is_empty());
    }

    #[test]
    fn detects_event_handler_without_unsubscribe() {
        let src = r#"
class MyForm {
    void Init() {
        button.Click += OnClick;
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "event_handler_leak")
            .collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].message.contains("button.Click"));
    }

    #[test]
    fn ignores_event_with_unsubscribe() {
        let src = r#"
class MyForm {
    void Init() {
        button.Click += OnClick;
    }
    void Cleanup() {
        button.Click -= OnClick;
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "event_handler_leak")
            .collect();
        assert!(matched.is_empty());
    }

    #[test]
    fn detects_collection_add_in_loop() {
        let src = r#"
class Cache {
    void Populate() {
        while (true) {
            items.Add(GetNext());
        }
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_collection_growth")
            .collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn detects_insert_in_foreach_loop() {
        let src = r#"
class Processor {
    void Process(List<Item> source) {
        foreach (var item in source) {
            results.Insert(0, item);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_collection_growth")
            .collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn ignores_add_outside_loop() {
        let src = r#"
class Service {
    void AddItem(Item item) {
        items.Add(item);
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_collection_growth")
            .collect();
        assert!(matched.is_empty());
    }
}
