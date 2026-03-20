use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{extract_receiver_text, receiver_matches_any};
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const EF_METHODS: &[&str] = &[
    "Find",
    "FindAsync",
    "FirstOrDefault",
    "FirstOrDefaultAsync",
    "Single",
    "SingleOrDefault",
    "SingleAsync",
    "Where",
    "ExecuteReader",
    "ExecuteNonQuery",
    "ExecuteScalar",
    "ExecuteReaderAsync",
    "ExecuteNonQueryAsync",
    "ExecuteScalarAsync",
];

const HTTP_METHODS: &[&str] = &[
    "GetAsync",
    "PostAsync",
    "PutAsync",
    "DeleteAsync",
    "SendAsync",
    "GetStringAsync",
    "GetStreamAsync",
    "GetByteArrayAsync",
];

const DB_OBJECT_CREATION_TYPES: &[&str] = &["SqlCommand", "SqlConnection"];

/// LINQ methods that are ambiguous — they can operate on in-memory collections
/// or on DB-backed IQueryable. We skip if the receiver looks like an in-memory collection.
const LINQ_AMBIGUOUS_METHODS: &[&str] = &[
    "Where",
    "Select",
    "FirstOrDefault",
    "FirstOrDefaultAsync",
    "Single",
    "SingleOrDefault",
    "SingleAsync",
    "Find",
    "FindAsync",
];

/// Receiver patterns that indicate in-memory collections (skip flagging).
const IN_MEMORY_RECEIVER_PATTERNS: &[&str] = &[
    "list",
    "array",
    "collection",
    "enumerable",
    "items",
    "results",
];

/// Receiver patterns that indicate DB context (require for ambiguous methods).
const DB_CONTEXT_RECEIVER_PATTERNS: &[&str] =
    &["context", "dbcontext", "dbset", "repository", "entities"];

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    invocation_query: Arc<Query>,
    object_creation_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
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
            .with_context(|| "failed to compile loop query for C# n_plus_one_queries")?;

        let invocation_query_str = r#"
(invocation_expression
  function: (_) @fn_expr
  arguments: (argument_list) @args) @invocation
"#;
        let invocation_query = Query::new(&csharp_lang(), invocation_query_str)
            .with_context(|| "failed to compile invocation query for C# n_plus_one_queries")?;

        let object_creation_query_str = r#"
(object_creation_expression
  type: (_) @type_name
  arguments: (argument_list) @args) @creation
"#;
        let object_creation_query = Query::new(&csharp_lang(), object_creation_query_str)
            .with_context(|| "failed to compile object_creation query for C# n_plus_one_queries")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            invocation_query: Arc::new(invocation_query),
            object_creation_query: Arc::new(object_creation_query),
        })
    }

    fn find_calls_in_body<'a>(
        &self,
        body_node: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Vec<(tree_sitter::Node<'a>, &'static str)> {
        let mut results = Vec::new();

        // Check invocation expressions for EF/ADO.NET and HTTP patterns
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.invocation_query, body_node, source);

            let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
            let inv_idx = find_capture_index(&self.invocation_query, "invocation");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_idx)
                    .map(|c| c.node);
                let inv_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == inv_idx)
                    .map(|c| c.node);

                if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                    let fn_text = node_text(fn_node, source);

                    // Check EF/ADO.NET patterns
                    for &method in EF_METHODS {
                        if fn_text.ends_with(method) {
                            // For LINQ-ambiguous methods, check the receiver to
                            // avoid false positives on in-memory collections.
                            if LINQ_AMBIGUOUS_METHODS.contains(&method) {
                                let receiver = extract_receiver_text(inv_node, source);
                                if !receiver.is_empty() {
                                    // Skip if receiver looks like an in-memory collection
                                    if receiver_matches_any(receiver, IN_MEMORY_RECEIVER_PATTERNS) {
                                        break;
                                    }
                                    // For ambiguous methods, only flag if receiver looks like a DB context
                                    if !receiver_matches_any(receiver, DB_CONTEXT_RECEIVER_PATTERNS)
                                    {
                                        break;
                                    }
                                }
                            }
                            results.push((inv_node, "db_query_in_loop"));
                            break;
                        }
                    }

                    // Check HTTP patterns
                    for &method in HTTP_METHODS {
                        if fn_text.ends_with(method) {
                            let already = results.iter().any(|(n, _)| n.id() == inv_node.id());
                            if !already {
                                results.push((inv_node, "http_call_in_loop"));
                            }
                            break;
                        }
                    }

                    // Check WebRequest.Create
                    if fn_text.ends_with("WebRequest.Create") {
                        let already = results.iter().any(|(n, _)| n.id() == inv_node.id());
                        if !already {
                            results.push((inv_node, "http_call_in_loop"));
                        }
                    }
                }
            }
        }

        // Check object creation for SqlCommand, DbContext etc. inside loops
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.object_creation_query, body_node, source);

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
                    if DB_OBJECT_CREATION_TYPES.contains(&type_text) {
                        results.push((creation_node, "db_query_in_loop"));
                    }
                }
            }
        }

        results
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects database queries and HTTP calls inside loops (N+1 query pattern)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                let calls = self.find_calls_in_body(body, source);

                for (call_node, pattern) in calls {
                    let start = call_node.start_position();
                    let call_text = node_text(call_node, source);
                    let kind = if pattern == "db_query_in_loop" {
                        "Database/ORM"
                    } else {
                        "HTTP"
                    };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: format!(
                            "{kind} call `{}` inside loop \u{2014} potential N+1 query problem",
                            call_text.lines().next().unwrap_or(call_text)
                        ),
                        snippet: extract_snippet(source, call_node, 2),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_ef_find_in_foreach_loop() {
        let src = r#"
class UserService {
    void LoadUsers(List<int> ids) {
        foreach (var id in ids) {
            context.Users.Find(id);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn detects_first_or_default_in_for_loop() {
        let src = r#"
class OrderService {
    void ProcessOrders(List<int> orderIds) {
        for (int i = 0; i < orderIds.Count; i++) {
            var order = context.Orders.FirstOrDefault(o => o.Id == orderIds[i]);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn detects_execute_reader_in_while_loop() {
        let src = r#"
class DataAccess {
    void Process() {
        int i = 0;
        while (i < 10) {
            cmd.ExecuteReader();
            i++;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn detects_http_get_async_in_loop() {
        let src = r#"
class Fetcher {
    async Task FetchAll(List<string> urls) {
        foreach (var url in urls) {
            await client.GetAsync(url);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "http_call_in_loop");
    }

    #[test]
    fn detects_sql_command_creation_in_loop() {
        let src = r#"
class DataAccess {
    void Process(List<int> ids) {
        foreach (var id in ids) {
            new SqlCommand("SELECT * FROM Users WHERE Id = " + id, conn);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "db_query_in_loop"));
    }

    #[test]
    fn ignores_query_outside_loop() {
        let src = r#"
class UserService {
    void GetUser(int id) {
        context.Users.Find(id);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_db_call_in_loop() {
        let src = r#"
class Foo {
    void Process(List<string> items) {
        foreach (var item in items) {
            Console.WriteLine(item);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
