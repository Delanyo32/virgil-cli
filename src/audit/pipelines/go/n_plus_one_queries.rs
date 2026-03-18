use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_for_statement_query, compile_method_call_query, compile_selector_call_query,
    extract_snippet, find_capture_index, node_text,
};

const DB_METHOD_NAMES: &[&str] = &[
    "QueryRow", "Query", "Exec", "Get", "Select", "Find", "First", "Where",
    "Create", "Update", "Delete", "Save", "Preload", "Joins",
];

const HTTP_METHOD_NAMES: &[&str] = &[
    "Get", "Post", "Do", "Put", "Patch", "Delete", "Head",
];

const HTTP_PACKAGE_NAMES: &[&str] = &["http", "client"];

pub struct NPlusOneQueriesPipeline {
    for_query: Arc<Query>,
    selector_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            for_query: compile_for_statement_query()?,
            selector_query: compile_selector_call_query()?,
            method_call_query: compile_method_call_query()?,
        })
    }

    fn find_db_calls_in_body<'a>(
        &self,
        body_node: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Vec<(tree_sitter::Node<'a>, &'static str)> {
        let mut results = Vec::new();
        let mut seen_call_starts = std::collections::HashSet::new();

        // Check selector calls (pkg.Method pattern) for http calls
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.selector_query, body_node, source);

        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches.next() {
            let pkg_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg_node, method_node, call_node) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                if HTTP_PACKAGE_NAMES.contains(&pkg_name)
                    && HTTP_METHOD_NAMES.contains(&method_name)
                {
                    seen_call_starts.insert(call.start_byte());
                    results.push((call, "http_call_in_loop"));
                }
            }
        }

        // Check method calls (.Method pattern) for DB/ORM calls
        let mut cursor2 = QueryCursor::new();
        let mut matches2 = cursor2.matches(&self.method_call_query, body_node, source);

        let method_name_idx = find_capture_index(&self.method_call_query, "method_name");
        let call2_idx = find_capture_index(&self.method_call_query, "call");

        while let Some(m) = matches2.next() {
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call2_idx)
                .map(|c| c.node);

            if let (Some(method), Some(call)) = (method_node, call_node) {
                // Skip calls already reported by the HTTP check (avoids double-counting)
                if seen_call_starts.contains(&call.start_byte()) {
                    continue;
                }
                let method_name = node_text(method, source);

                if DB_METHOD_NAMES.contains(&method_name) {
                    results.push((call, "db_query_in_loop"));
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
        let mut matches = cursor.matches(&self.for_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.for_query, "for_body");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let calls = self.find_db_calls_in_body(body, source);

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
                        snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_db_query_in_loop() {
        let src = r#"package main

func getUsers(db *sql.DB, ids []int) {
	for _, id := range ids {
		db.QueryRow("SELECT * FROM users WHERE id = ?", id)
	}
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn detects_orm_call_in_loop() {
        let src = r#"package main

func loadPosts(db *gorm.DB, userIDs []int) {
	for _, uid := range userIDs {
		db.Where("user_id = ?", uid).Find(&posts)
	}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "db_query_in_loop"));
    }

    #[test]
    fn detects_http_call_in_loop() {
        let src = r#"package main

import "net/http"

func fetchAll(urls []string) {
	for _, url := range urls {
		http.Get(url)
	}
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "http_call_in_loop");
    }

    #[test]
    fn ignores_query_outside_loop() {
        let src = r#"package main

func getUser(db *sql.DB, id int) {
	db.QueryRow("SELECT * FROM users WHERE id = ?", id)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_db_call_in_loop() {
        let src = r#"package main

func process(items []int) {
	for _, item := range items {
		fmt.Println(item)
	}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
