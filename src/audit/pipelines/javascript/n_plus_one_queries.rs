use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::pipelines::helpers::{extract_receiver_text, receiver_matches_any};

/// DB/ORM method names that suggest a database query.
const DB_METHOD_NAMES: &[&str] = &[
    "findOne",
    "findUnique",
    "findMany",
    "find",
    "findById",
    "query",
    "execute",
    "get",
];

/// Generic method names that are also common on non-DB types (arrays, maps, etc.).
/// These require receiver checking to avoid false positives.
const GENERIC_METHOD_NAMES: &[&str] = &["find", "get"];

/// Receiver patterns that indicate non-DB usage (case-insensitive).
const NON_DB_RECEIVERS: &[&str] = &[
    "arr", "array", "list", "map", "set", "cache", "store", "items", "collection", "data",
];

/// Receiver patterns that confirm DB usage (case-insensitive).
#[allow(dead_code)]
const DB_RECEIVERS: &[&str] = &[
    "db", "conn", "model", "repository", "prisma", "sequelize", "mongoose", "knex",
];

/// Object.method patterns that suggest a database or HTTP call.
const DB_OBJ_METHOD_PAIRS: &[(&str, &str)] = &[
    ("Model", "find"),
    ("db", "collection"),
    ("axios", "get"),
    ("axios", "post"),
    ("axios", "put"),
    ("axios", "delete"),
    ("axios", "patch"),
    ("http", "get"),
    ("http", "post"),
    ("http", "request"),
];

/// Bare function calls that suggest network access.
const BARE_CALL_PATTERNS: &[&str] = &["fetch", "request"];

/// Array iteration methods that act as implicit loops.
const ARRAY_LOOP_METHODS: &[&str] = &["forEach", "map", "flatMap", "filter", "reduce", "some", "every"];

/// Traditional loop node kinds.
const LOOP_KINDS: &[&str] = &[
    "for_statement",
    "for_in_statement",
    "while_statement",
    "do_statement",
];

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    method_call_query: Arc<Query>,
    direct_call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let method_call_str = r#"
(call_expression
  function: (member_expression
    object: (_) @obj
    property: (property_identifier) @method)
  arguments: (arguments) @args) @call
"#;
        let direct_call_str = r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args) @call
"#;
        Ok(Self {
            method_call_query: Arc::new(
                Query::new(&js_lang(), method_call_str)
                    .with_context(|| "failed to compile method_call query for n_plus_one")?,
            ),
            direct_call_query: Arc::new(
                Query::new(&js_lang(), direct_call_str)
                    .with_context(|| "failed to compile direct_call query for n_plus_one")?,
            ),
        })
    }

    /// Returns true if `node` is inside a traditional loop body.
    fn is_inside_loop(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if LOOP_KINDS.contains(&parent.kind()) {
                return true;
            }
            current = parent.parent();
        }
        false
    }

    /// Returns true if `node` is inside a callback passed to an array iteration method
    /// (e.g. `.forEach(callback)`, `.map(callback)`).
    fn is_inside_array_method_callback(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            // Look for arrow_function or function_expression whose parent is arguments
            // whose parent is a call_expression with a matching method name.
            if parent.kind() == "arrow_function" || parent.kind() == "function_expression" {
                if let Some(args_node) = parent.parent() {
                    if args_node.kind() == "arguments" {
                        if let Some(call_node) = args_node.parent() {
                            if call_node.kind() == "call_expression" {
                                if let Some(func) = call_node.child_by_field_name("function") {
                                    if func.kind() == "member_expression" {
                                        if let Some(prop) =
                                            func.child_by_field_name("property")
                                        {
                                            let method_name = node_text(prop, source);
                                            if ARRAY_LOOP_METHODS.contains(&method_name) {
                                                return true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            current = parent.parent();
        }
        false
    }

    /// Returns true if `node` is inside a loop-like context (traditional loop or array method callback).
    fn is_in_loop_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        Self::is_inside_loop(node) || Self::is_inside_array_method_callback(node, source)
    }

    /// Check if a node has an `await_expression` ancestor between it and the loop boundary.
    fn has_await_ancestor(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "await_expression" {
                return true;
            }
            if LOOP_KINDS.contains(&parent.kind()) {
                break;
            }
            current = parent.parent();
        }
        false
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

        // Check method calls (obj.method patterns)
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_call_query, tree.root_node(), source);
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
                let obj_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == obj_idx)
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

                if let (Some(obj), Some(method), Some(call)) = (obj_node, method_node, call_node) {
                    if !Self::is_in_loop_context(call, source) {
                        continue;
                    }

                    let obj_name = node_text(obj, source);
                    let method_name = node_text(method, source);

                    // Check specific obj.method pairs
                    let mut matched_pair = false;
                    for &(expected_obj, expected_method) in DB_OBJ_METHOD_PAIRS {
                        if obj_name == expected_obj && method_name == expected_method {
                            let has_await = Self::has_await_ancestor(call);
                            let await_hint = if has_await { " (awaited)" } else { "" };
                            findings.push(self.make_finding(
                                call,
                                source,
                                file_path,
                                &format!(
                                    "`{obj_name}.{method_name}()` called inside a loop{await_hint} — potential N+1 query"
                                ),
                            ));
                            matched_pair = true;
                            break;
                        }
                    }
                    if matched_pair {
                        continue;
                    }

                    // Check generic DB method names on any receiver
                    if DB_METHOD_NAMES.contains(&method_name) {
                        // For generic method names like .find(), .get(), check the receiver
                        // to avoid false positives on array/collection operations
                        if GENERIC_METHOD_NAMES.contains(&method_name) {
                            let receiver = extract_receiver_text(call, source);
                            if !receiver.is_empty() && receiver_matches_any(receiver, NON_DB_RECEIVERS) {
                                continue;
                            }
                        }
                        let has_await = Self::has_await_ancestor(call);
                        let await_hint = if has_await { " (awaited)" } else { "" };
                        findings.push(self.make_finding(
                            call,
                            source,
                            file_path,
                            &format!(
                                "`.{method_name}()` called inside a loop{await_hint} — potential N+1 query"
                            ),
                        ));
                    }
                }
            }
        }

        // Check bare function calls (fetch, request)
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_name_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_n), Some(call)) = (fn_node, call_node) {
                    if !Self::is_in_loop_context(call, source) {
                        continue;
                    }

                    let fn_name = node_text(fn_n, source);
                    if BARE_CALL_PATTERNS.contains(&fn_name) {
                        let has_await = Self::has_await_ancestor(call);
                        let await_hint = if has_await { " (awaited)" } else { "" };
                        findings.push(self.make_finding(
                            call,
                            source,
                            file_path,
                            &format!(
                                "`{fn_name}()` called inside a loop{await_hint} — potential N+1 query"
                            ),
                        ));
                    }
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_findone_in_for_loop() {
        let src = "\
for (let i = 0; i < ids.length; i++) {
    const user = await User.findOne({ id: ids[i] });
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "query_in_loop");
        assert!(findings[0].message.contains("findOne"));
    }

    #[test]
    fn detects_fetch_in_for_of_loop() {
        let src = "\
for (const url of urls) {
    const res = await fetch(url);
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fetch"));
    }

    #[test]
    fn detects_axios_get_in_while_loop() {
        let src = "\
while (hasMore) {
    const data = await axios.get('/api/items');
    hasMore = data.next;
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("axios.get"));
    }

    #[test]
    fn detects_query_in_foreach_callback() {
        let src = "\
items.forEach(async (item) => {
    await db.collection('items').findOne({ id: item.id });
});";
        let findings = parse_and_check(src);
        assert!(findings.len() >= 1);
    }

    #[test]
    fn ignores_call_outside_loop() {
        let src = "\
const user = await User.findOne({ id: 1 });
const res = await fetch('/api/data');";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_db_execute_in_do_while() {
        let src = "\
do {
    db.execute('SELECT * FROM users WHERE id = ?', [id]);
    id++;
} while (id < 100);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("execute"));
    }

    #[test]
    fn ignores_non_db_call_in_loop() {
        let src = "\
for (const item of items) {
    console.log(item);
    process(item);
}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
