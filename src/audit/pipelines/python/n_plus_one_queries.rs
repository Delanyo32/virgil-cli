use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{Pipeline, PipelineContext};
use crate::audit::pipelines::helpers::{extract_receiver_text, receiver_matches_any_word};

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
const BARE_CALL_PATTERNS: &[&str] = &["urlopen"];

/// Attribute call patterns where any object can be the receiver (e.g. `urllib.request.urlopen`).
const DEEP_ATTR_TAILS: &[&str] = &["urlopen"];

/// Non-DB receiver patterns — skip these for ambiguous methods like .filter(), .get(), etc.
const NON_DB_RECEIVERS: &[&str] = &[
    "list", "dict", "set", "cache", "arr", "tuple", "str", "iter",
];

/// DB receiver patterns — require these for ambiguous methods
const DB_RECEIVERS: &[&str] = &[
    "session", "query", "db", "model", "objects", "cursor", "conn",
];

/// Batch methods — skip these, they are the opposite of N+1
const BATCH_METHODS: &[&str] = &["bulk_create", "bulk_update", "in_bulk"];

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
            let obj = fn_node
                .child_by_field_name("object")
                .map(|n| node_text(n, source));
            let attr = fn_node
                .child_by_field_name("attribute")
                .map(|n| node_text(n, source));

            if let (Some(obj), Some(attr)) = (obj, attr) {
                // Skip batch methods — they are the fix for N+1
                if BATCH_METHODS.contains(&attr) {
                    return None;
                }

                // Check specific obj.method patterns
                for &(expected_obj, expected_method) in DB_ATTR_CALLS {
                    if obj == expected_obj && attr == expected_method {
                        return Some(self.make_finding(
                            call_node,
                            source,
                            file_path,
                            &format!("`{obj}.{attr}()` called inside a loop — potential N+1 query"),
                        ));
                    }
                }

                // Check ORM method names with receiver validation
                if ORM_METHOD_NAMES.contains(&attr) {
                    let receiver = extract_receiver_text(call_node, source);
                    // Skip if receiver matches non-DB patterns
                    if !receiver.is_empty() && receiver_matches_any_word(receiver, NON_DB_RECEIVERS)
                    {
                        return None;
                    }
                    // Only flag if receiver matches DB patterns or is unknown
                    if receiver.is_empty() || receiver_matches_any_word(receiver, DB_RECEIVERS) {
                        return Some(self.make_finding(
                            call_node,
                            source,
                            file_path,
                            &format!("`.{attr}()` called inside a loop — potential N+1 query"),
                        ));
                    }
                    // No pattern match, skip (conservative)
                    return None;
                }

                // Check deep attribute tails (e.g. urllib.request.urlopen)
                if DEEP_ATTR_TAILS.contains(&attr) {
                    return Some(self.make_finding(
                        call_node,
                        source,
                        file_path,
                        &format!("`{fn_text}()` called inside a loop — potential N+1 query"),
                    ));
                }
            }
        } else if fn_node.kind() == "identifier" {
            // Bare function calls like urlopen(...)
            if BARE_CALL_PATTERNS.contains(&fn_text) {
                return Some(self.make_finding(
                    call_node,
                    source,
                    file_path,
                    &format!("`{fn_text}()` called inside a loop — potential N+1 query"),
                ));
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

    /// Find a `call` node at the given 0-indexed row in the tree.
    fn find_call_node_at_row(
        root: tree_sitter::Node,
        row: usize,
    ) -> Option<tree_sitter::Node> {
        let mut cursor = root.walk();
        let mut result = None;
        Self::walk_tree_for_call(root, &mut cursor, row, &mut result);
        result
    }

    fn walk_tree_for_call<'a>(
        node: tree_sitter::Node<'a>,
        cursor: &mut tree_sitter::TreeCursor<'a>,
        row: usize,
        result: &mut Option<tree_sitter::Node<'a>>,
    ) {
        if result.is_some() {
            return;
        }
        if node.kind() == "call" && node.start_position().row == row {
            *result = Some(node);
            return;
        }
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                // Only recurse if the child's row range overlaps our target
                if child.start_position().row <= row && child.end_position().row >= row {
                    Self::walk_tree_for_call(child, cursor, row, result);
                    if result.is_some() {
                        // Restore cursor position before returning
                        cursor.goto_parent();
                        return;
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    /// Walk up from a node to find the enclosing function definition.
    fn find_enclosing_function(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition" {
                return Some(parent);
            }
            current = parent.parent();
        }
        None
    }

    /// Check if a receiver variable is assigned from a non-DB source within the
    /// enclosing function body. Returns `true` if the receiver is definitely
    /// non-DB (i.e., the finding should be suppressed).
    fn receiver_assigned_from_non_db(
        func_node: tree_sitter::Node,
        receiver_name: &str,
        source: &[u8],
    ) -> bool {
        // Find the function body (block child)
        let body = func_node.child_by_field_name("body");
        let body = match body {
            Some(b) => b,
            None => return false,
        };

        let mut cursor = body.walk();
        Self::scan_assignments_for_non_db(body, &mut cursor, receiver_name, source)
    }

    fn scan_assignments_for_non_db<'a>(
        node: tree_sitter::Node<'a>,
        cursor: &mut tree_sitter::TreeCursor<'a>,
        receiver_name: &str,
        source: &[u8],
    ) -> bool {
        if node.kind() == "assignment" {
            // Check if the left side matches receiver_name
            if let Some(left) = node.child_by_field_name("left") {
                let left_text = left.utf8_text(source).unwrap_or("");
                if left_text == receiver_name {
                    // Check the right-hand side
                    if let Some(right) = node.child_by_field_name("right") {
                        if Self::is_non_db_expression(right, source) {
                            return true;
                        }
                    }
                }
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if Self::scan_assignments_for_non_db(child, cursor, receiver_name, source) {
                    cursor.goto_parent();
                    return true;
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
        false
    }

    /// Returns true if the expression is clearly a non-DB value
    /// (dict/list/set literal, comprehension, or dict()/list()/set() call).
    fn is_non_db_expression(node: tree_sitter::Node, source: &[u8]) -> bool {
        let kind = node.kind();
        match kind {
            // Literals: {}, [], set()
            "dictionary" | "list" | "set" => true,
            // Comprehensions
            "dictionary_comprehension" | "list_comprehension" | "set_comprehension"
            | "generator_expression" => true,
            // Subscript on a variable (e.g. data[key]) — likely dict/list access
            "subscript" => true,
            // Call to dict(), list(), set()
            "call" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let func_text = func.utf8_text(source).unwrap_or("");
                    matches!(func_text, "dict" | "list" | "set" | "tuple" | "frozenset")
                } else {
                    false
                }
            }
            // Tuple literal
            "tuple" => true,
            _ => false,
        }
    }

    /// Check if a `.get()` call has 2+ arguments (key + default), which is
    /// a strong signal it's a dict `.get(key, default)`.
    fn is_dict_get_with_default(
        call_node: tree_sitter::Node,
        source: &[u8],
    ) -> bool {
        // The call should be an attribute call where method is "get"
        if let Some(func) = call_node.child_by_field_name("function") {
            if func.kind() == "attribute" {
                if let Some(attr) = func.child_by_field_name("attribute") {
                    let method = attr.utf8_text(source).unwrap_or("");
                    if method == "get" {
                        // Count arguments in argument_list
                        if let Some(args) = call_node.child_by_field_name("arguments") {
                            let mut arg_count = 0;
                            let mut arg_cursor = args.walk();
                            if arg_cursor.goto_first_child() {
                                loop {
                                    let child = arg_cursor.node();
                                    // Skip punctuation (parens, commas)
                                    if child.is_named() {
                                        arg_count += 1;
                                    }
                                    if !arg_cursor.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                            // .get(key, default) has 2 args
                            return arg_count >= 2;
                        }
                    }
                }
            }
        }
        false
    }

    /// Apply tree-sitter-based assignment filtering to reduce false positives.
    /// Returns findings with non-DB receivers suppressed.
    fn filter_findings_via_tree(
        findings: Vec<AuditFinding>,
        tree: &Tree,
        source: &[u8],
    ) -> Vec<AuditFinding> {
        findings
            .into_iter()
            .filter(|finding| {
                let row = (finding.line - 1) as usize; // 0-indexed

                // Find the call node at this line
                let call_node = match Self::find_call_node_at_row(tree.root_node(), row) {
                    Some(n) => n,
                    None => return true, // Keep finding if we can't locate the call
                };

                // Check if it's a .get() with a default value — almost certainly a dict
                if Self::is_dict_get_with_default(call_node, source) {
                    return false; // Suppress
                }

                // Extract receiver
                let receiver = extract_receiver_text(call_node, source);
                if receiver.is_empty() {
                    return true; // Keep: no receiver to check
                }

                // Find enclosing function and check assignments
                if let Some(func_node) = Self::find_enclosing_function(call_node) {
                    if Self::receiver_assigned_from_non_db(func_node, receiver, source) {
                        return false; // Suppress: assigned from non-DB source
                    }
                }

                // Check parameter names against NON_DB_RECEIVERS
                if receiver_matches_any_word(receiver, NON_DB_RECEIVERS) {
                    return false; // Suppress
                }

                true // Keep
            })
            .collect()
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects DB/ORM/HTTP calls inside loops (N+1 query pattern)"
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        let base_findings = self.check(ctx.tree, ctx.source, ctx.file_path);
        // Apply tree-sitter-based assignment filtering regardless of graph availability.
        // The graph could provide additional context in the future, but the
        // assignment + dict-get heuristics work purely from the syntax tree.
        Self::filter_findings_via_tree(base_findings, ctx.tree, ctx.source)
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

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
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        let id_counts = HashMap::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: None,
        };
        pipeline.check_with_context(&ctx)
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
        assert!(!findings.is_empty());
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

    #[test]
    fn suppresses_dict_get_with_default_in_loop() {
        // db.get(key, default) — .get() with 2 args is dict pattern even on DB-named receiver
        let src = "\
def process(data):
    for key in keys:
        value = db.get(key, None)
";
        // Base check() would flag this (db matches DB_RECEIVERS)
        let base = parse_and_check(src);
        assert_eq!(base.len(), 1, "base check should flag db.get()");
        // But check_with_context suppresses because of 2-arg .get() heuristic
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "dict .get(key, default) should not be flagged, got: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn suppresses_variable_assigned_from_dict_literal() {
        // db_lookup = {} then db_lookup.get(id) — receiver has "db" but assigned from dict literal
        let src = "\
def process(items):
    db_lookup = {}
    for item in items:
        db_lookup.get(item.id)
";
        // Base check() would flag this (db_lookup contains "db" → matches DB_RECEIVERS)
        let base = parse_and_check(src);
        assert_eq!(base.len(), 1, "base check should flag db_lookup.get()");
        // check_with_context suppresses because db_lookup is assigned from dict literal
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "variable assigned from dict literal should not be flagged, got: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn suppresses_variable_assigned_from_list_literal() {
        let src = "\
def process(data):
    results = []
    for item in data:
        results.filter(lambda x: x > item)
";
        // results doesn't match DB or NON_DB, so check() skips it (conservative)
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "variable assigned from list literal should not be flagged, got: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn suppresses_variable_assigned_from_dict_call() {
        // db_map = dict() then db_map.get(key) — receiver has "db" but assigned from dict()
        let src = "\
def process(items):
    db_map = dict()
    for item in items:
        db_map.get(item.key)
";
        // Base check() would flag this (db_map contains "db" → matches DB_RECEIVERS)
        let base = parse_and_check(src);
        assert_eq!(base.len(), 1, "base check should flag db_map.get()");
        // check_with_context suppresses because db_map is assigned from dict()
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "variable assigned from dict() call should not be flagged, got: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn suppresses_variable_assigned_from_comprehension() {
        // db_index assigned from dict comprehension
        let src = "\
def process(data):
    db_index = {item.id: item for item in data}
    for key in keys:
        db_index.get(key)
";
        // Base check() would flag this (db_index contains "db" → matches DB_RECEIVERS)
        let base = parse_and_check(src);
        assert_eq!(base.len(), 1, "base check should flag db_index.get()");
        // check_with_context suppresses because db_index is assigned from comprehension
        let findings = parse_and_check_with_context(src);
        assert!(
            findings.is_empty(),
            "variable assigned from dict comprehension should not be flagged, got: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn still_flags_genuine_db_query_in_loop() {
        let src = "\
def fetch_users(user_ids):
    for user_id in user_ids:
        cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))
";
        let findings = parse_and_check_with_context(src);
        assert_eq!(
            findings.len(),
            1,
            "genuine DB query should still be flagged"
        );
        assert!(findings[0].message.contains("cursor.execute"));
    }

    #[test]
    fn still_flags_db_receiver_get_in_loop() {
        let src = "\
def fetch_data(ids):
    for id in ids:
        db.get(id)
";
        let findings = parse_and_check_with_context(src);
        assert_eq!(
            findings.len(),
            1,
            "db.get() with single arg should still be flagged"
        );
    }
}
