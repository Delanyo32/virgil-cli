use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const DB_METHODS: &[&str] = &[
    "find",
    "findById",
    "createQuery",
    "executeQuery",
    "executeUpdate",
    "prepareStatement",
    "getResultSet",
];

const DB_CHAINED_CALLS: &[(&str, &str)] = &[
    ("entityManager", "find"),
    ("session", "get"),
    ("session", "load"),
];

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_statement
    body: (_) @loop_body) @loop
  (enhanced_for_statement
    body: (_) @loop_body) @loop
  (while_statement
    body: (_) @loop_body) @loop
  (do_statement
    body: (_) @loop_body) @loop
]
"#;
        let loop_query = Query::new(&java_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for Java n_plus_one_queries")?;

        let method_query_str = r#"
(method_invocation
  object: (_)? @object
  name: (identifier) @method_name
  arguments: (argument_list) @args) @invocation
"#;
        let method_query = Query::new(&java_lang(), method_query_str)
            .with_context(|| "failed to compile method_invocation query for Java n_plus_one_queries")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            method_query: Arc::new(method_query),
        })
    }

    fn find_calls_in_body<'a>(
        &self,
        body_node: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Vec<(tree_sitter::Node<'a>, &'static str)> {
        let mut results = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, body_node, source);

        let object_idx = find_capture_index(&self.method_query, "object");
        let method_idx = find_capture_index(&self.method_query, "method_name");
        let invocation_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let object_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == object_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == invocation_idx)
                .map(|c| c.node);

            if let (Some(method_node), Some(inv_node)) = (method_node, inv_node) {
                let method_name = node_text(method_node, source);

                // Check chained calls like entityManager.find, session.get, session.load
                if let Some(obj) = object_node {
                    let obj_text = node_text(obj, source);
                    for &(obj_pattern, method_pattern) in DB_CHAINED_CALLS {
                        if obj_text == obj_pattern && method_name == method_pattern {
                            results.push((inv_node, "db_query_in_loop"));
                        }
                    }

                    // Check HTTP patterns
                    if method_name == "send" && obj_text.contains("HttpClient")
                        || obj_text.contains("httpClient")
                    {
                        results.push((inv_node, "http_call_in_loop"));
                        continue;
                    }
                    if method_name == "openConnection" {
                        results.push((inv_node, "http_call_in_loop"));
                        continue;
                    }
                    if method_name == "getForObject"
                        || method_name == "getForEntity"
                        || method_name == "postForObject"
                        || method_name == "exchange"
                    {
                        results.push((inv_node, "http_call_in_loop"));
                        continue;
                    }
                }

                // Check simple DB method names
                if DB_METHODS.contains(&method_name) {
                    // Avoid duplicates from chained calls already matched
                    let already_found = results.iter().any(|(n, _)| n.id() == inv_node.id());
                    if !already_found {
                        results.push((inv_node, "db_query_in_loop"));
                    }
                }

                // Check HTTP send without object context
                if method_name == "send" && object_node.is_none() {
                    // Could be HttpClient.send style
                    let inv_text = node_text(inv_node, source);
                    if inv_text.contains("HttpClient") || inv_text.contains("httpClient") {
                        results.push((inv_node, "http_call_in_loop"));
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
                            "{kind} call `{}` inside loop — potential N+1 query problem",
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_db_query_in_for_loop() {
        let src = r#"class UserService {
    void loadUsers(List<Long> ids) {
        for (Long id : ids) {
            repo.findById(id);
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn detects_execute_query_in_while_loop() {
        let src = r#"class Dao {
    void process() {
        int i = 0;
        while (i < 10) {
            stmt.executeQuery("SELECT * FROM items WHERE id = " + i);
            i++;
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn detects_entity_manager_find_in_loop() {
        let src = r#"class UserService {
    void loadUsers(List<Long> ids) {
        for (Long id : ids) {
            entityManager.find(User.class, id);
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "db_query_in_loop"));
    }

    #[test]
    fn detects_http_call_in_loop() {
        let src = r#"class Fetcher {
    void fetchAll(List<String> urls) {
        for (String url : urls) {
            restTemplate.getForObject(url, String.class);
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "http_call_in_loop");
    }

    #[test]
    fn ignores_query_outside_loop() {
        let src = r#"class Dao {
    void getUser(Long id) {
        repo.findById(id);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_db_call_in_loop() {
        let src = r#"class Foo {
    void process(List<String> items) {
        for (String item : items) {
            System.out.println(item);
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
