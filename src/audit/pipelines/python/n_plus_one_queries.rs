use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{extract_receiver_text, receiver_matches_any};

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

/// Call patterns that suggest a DB/ORM/HTTP query.
const DB_ATTR_CALLS: &[(&str, &str)] = &[
    ("cursor", "execute"),
    ("session", "query"),
    ("requests", "get"),
    ("requests", "post"),
    ("requests", "put"),
    ("requests", "delete"),
    ("requests", "patch"),
    ("httpx", "get"),
    ("httpx", "post"),
    ("httpx", "put"),
    ("httpx", "delete"),
];

/// Method names on arbitrary objects that suggest a DB/ORM query.
const ORM_METHOD_NAMES: &[&str] = &[
    "filter", "get", "execute", "fetchone", "fetchall", "all", "first",
];

/// Bare function calls that suggest network/DB access.
const BARE_CALL_PATTERNS: &[&str] = &[
    "urlopen",
];

/// Attribute call patterns where any object can be the receiver (e.g. `urllib.request.urlopen`).
const DEEP_ATTR_TAILS: &[&str] = &[
    "urlopen",
];

/// Non-DB receiver patterns — skip these for ambiguous methods like .filter(), .get(), etc.
const NON_DB_RECEIVERS: &[&str] = &[
    "list", "dict", "set", "cache", "arr", "tuple", "str", "iter",
];

/// DB receiver patterns — require these for ambiguous methods
const DB_RECEIVERS: &[&str] = &[
    "session", "query", "db", "model", "objects", "cursor", "conn",
];

const LOOP_KINDS: &[&str] = &["for_statement", "while_statement"];

pub struct NPlusOneQueriesPipeline {
    call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }

    /// Returns true if `node` is a descendant of a loop (for/while).
    fn is_inside_loop(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if LOOP_KINDS.contains(&parent.kind()) {
                // Make sure we are inside the body of the loop, not in the loop header.
                // For `for_statement`, the body is the `block` child.
                // We simply check that node is a descendant (which it is, since we walked up).
                return true;
            }
            current = parent.parent();
        }
        false
    }

    /// Check whether a call node inside a loop matches a known DB/HTTP pattern.
    fn check_call(
        &self,
        fn_node: tree_sitter::Node,
        call_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
    ) -> Option<AuditFinding> {
        let fn_text = node_text(fn_node, source);

        if fn_node.kind() == "attribute" {
            let obj = fn_node.child_by_field_name("object").map(|n| node_text(n, source));
            let attr = fn_node.child_by_field_name("attribute").map(|n| node_text(n, source));

            if let (Some(obj), Some(attr)) = (obj, attr) {
                // Check specific obj.method patterns
                for &(expected_obj, expected_method) in DB_ATTR_CALLS {
                    if obj == expected_obj && attr == expected_method {
                        return Some(self.make_finding(call_node, source, file_path, &format!(
                            "`{obj}.{attr}()` called inside a loop — potential N+1 query"
                        )));
                    }
                }

                // Check ORM method names with receiver validation
                if ORM_METHOD_NAMES.contains(&attr) {
                    let receiver = extract_receiver_text(call_node, source);
                    // Skip if receiver matches non-DB patterns
                    if !receiver.is_empty() && receiver_matches_any(receiver, NON_DB_RECEIVERS) {
                        return None;
                    }
                    // Only flag if receiver matches DB patterns or is unknown
                    if receiver.is_empty() || receiver_matches_any(receiver, DB_RECEIVERS) {
                        return Some(self.make_finding(call_node, source, file_path, &format!(
                            "`.{attr}()` called inside a loop — potential N+1 query"
                        )));
                    }
                    // No pattern match, skip (conservative)
                    return None;
                }

                // Check deep attribute tails (e.g. urllib.request.urlopen)
                if DEEP_ATTR_TAILS.contains(&attr) {
                    return Some(self.make_finding(call_node, source, file_path, &format!(
                        "`{fn_text}()` called inside a loop — potential N+1 query"
                    )));
                }
            }
        } else if fn_node.kind() == "identifier" {
            // Bare function calls like urlopen(...)
            if BARE_CALL_PATTERNS.contains(&fn_text) {
                return Some(self.make_finding(call_node, source, file_path, &format!(
                    "`{fn_text}()` called inside a loop — potential N+1 query"
                )));
            }
        }

        None
    }

    fn make_finding(
        &self,
        call_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        message: &str,
    ) -> AuditFinding {
        let start = call_node.start_position();
        AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "warning".to_string(),
            pipeline: self.name().to_string(),
            pattern: "query_in_loop".to_string(),
            message: message.to_string(),
            snippet: extract_snippet(source, call_node, 1),
        }
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects DB/ORM/HTTP calls inside loops (N+1 query pattern)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m.captures.iter().find(|c| c.index as usize == fn_expr_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(fn_node), Some(call_node)) = (fn_node, call_node) {
                if !Self::is_inside_loop(call_node) {
                    continue;
                }

                if let Some(finding) = self.check_call(fn_node, call_node, source, file_path) {
                    findings.push(finding);
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_cursor_execute_in_for_loop() {
        let src = "\
for user_id in user_ids:
    cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "query_in_loop");
        assert!(findings[0].message.contains("cursor.execute"));
    }

    #[test]
    fn detects_orm_filter_in_while_loop() {
        let src = "\
while items:
    item = items.pop()
    result = db.filter(name=item)
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains(".filter()"));
    }

    #[test]
    fn detects_requests_get_in_loop() {
        let src = "\
for url in urls:
    resp = requests.get(url)
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("requests.get"));
    }

    #[test]
    fn ignores_call_outside_loop() {
        let src = "\
cursor.execute(\"SELECT * FROM users\")
result = db.filter(active=True)
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_nested_loop_call() {
        let src = "\
for group in groups:
    for user in group.users:
        session.query(User).get(user.id)
";
        let findings = parse_and_check(src);
        assert!(findings.len() >= 1);
    }

    #[test]
    fn ignores_non_db_call_in_loop() {
        let src = "\
for item in items:
    print(item)
    result = process(item)
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
