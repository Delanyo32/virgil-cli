use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use tree_sitter::Node;

/// Language-specific configuration for control flow analysis.
pub struct ControlFlowConfig {
    /// Node kinds that count as decision points for cyclomatic complexity
    /// (if, for, while, do, catch, case, etc.)
    pub decision_point_kinds: &'static [&'static str],
    /// Node kinds that increment cognitive complexity AND add nesting
    /// (if, for, while, do, switch, catch, etc.)
    pub nesting_increments: &'static [&'static str],
    /// Node kinds that increment cognitive complexity WITHOUT adding nesting
    /// (else if / elif, goto, break-to-label, etc.)
    pub flat_increments: &'static [&'static str],
    /// Logical operator tokens: "&&", "||", "and", "or"
    pub logical_operators: &'static [&'static str],
    /// The node kind for binary expressions containing logical operators
    pub binary_expression_kind: &'static str,
    /// The node kind for ternary/conditional expressions (None if language has none)
    pub ternary_kind: Option<&'static str>,
    /// Node kinds that represent comments
    pub comment_kinds: &'static [&'static str],
}

/// Compute cyclomatic complexity for a function body node.
///
/// CC = 1 + number of decision points + number of logical operators + ternaries
pub fn compute_cyclomatic(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize {
    let mut complexity: usize = 1;

    let mut cursor = body.walk();
    walk_all(body, &mut cursor, &mut |node| {
        let kind = node.kind();

        // Decision points
        if config.decision_point_kinds.contains(&kind) {
            complexity += 1;
        }

        // Ternary expressions
        if let Some(ternary) = config.ternary_kind
            && kind == ternary
        {
            complexity += 1;
        }

        // Logical operators in binary expressions
        if kind == config.binary_expression_kind
            && let Some(op_node) = node.child_by_field_name("operator")
        {
            let op_text = op_node.utf8_text(source).unwrap_or("");
            if config.logical_operators.contains(&op_text) {
                complexity += 1;
            }
        }
    });

    complexity
}

/// Compute cognitive complexity for a function body node.
///
/// Increments for each control flow break. Nesting increments also add
/// a penalty equal to the current nesting depth.
/// Uses stack-based iteration to avoid stack overflow on deeply nested ASTs.
pub fn compute_cognitive(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize {
    let mut score: usize = 0;
    let mut stack: Vec<(Node, usize)> = Vec::new();
    // Seed stack with body's direct children at nesting depth 0 (reverse for L-to-R order)
    let mut cursor = body.walk();
    let children: Vec<_> = body.children(&mut cursor).collect();
    for child in children.into_iter().rev() {
        stack.push((child, 0));
    }
    while let Some((node, nesting)) = stack.pop() {
        let kind = node.kind();
        let (increment, next_nesting) = if config.nesting_increments.contains(&kind) {
            (1 + nesting, nesting + 1)
        } else if config.flat_increments.contains(&kind) {
            (1, nesting)
        } else if config.ternary_kind == Some(kind) {
            (1 + nesting, nesting + 1)
        } else if kind == config.binary_expression_kind {
            if let Some(op_node) = node.child_by_field_name("operator") {
                let op_text = op_node.utf8_text(source).unwrap_or("");
                if config.logical_operators.contains(&op_text) {
                    (1, nesting)
                } else {
                    (0, nesting)
                }
            } else {
                (0, nesting)
            }
        } else {
            (0, nesting)
        };
        score += increment;
        let mut child_cursor = node.walk();
        let node_children: Vec<_> = node.children(&mut child_cursor).collect();
        for child in node_children.into_iter().rev() {
            stack.push((child, next_nesting));
        }
    }
    score
}

/// Count lines and statements in a function body.
///
/// Returns (total_lines, statement_count).
pub fn count_function_lines(body: Node) -> (usize, usize) {
    let start_line = body.start_position().row;
    let end_line = body.end_position().row;
    let total_lines = if end_line >= start_line {
        end_line - start_line + 1
    } else {
        1
    };

    let statement_count = count_statements(body);

    (total_lines, statement_count)
}

fn count_statements(root: Node) -> usize {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if kind.ends_with("_statement")
            || kind.ends_with("_declaration")
            || kind == "expression_statement"
            || kind == "return_statement"
            || kind == "throw_statement"
            || kind == "break_statement"
            || kind == "continue_statement"
        {
            count += 1;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    count
}

/// Compute comment-to-code ratio for the entire file root node.
///
/// Returns (comment_lines, code_lines). Code lines = total non-blank lines minus comment lines.
pub fn compute_comment_ratio(
    root: Node,
    source: &[u8],
    config: &ControlFlowConfig,
) -> (usize, usize) {
    let source_str = std::str::from_utf8(source).unwrap_or("");
    let total_non_blank: usize = source_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    let mut comment_lines: usize = 0;
    let mut cursor = root.walk();
    walk_all(root, &mut cursor, &mut |node| {
        if config.comment_kinds.contains(&node.kind()) {
            let start = node.start_position().row;
            let end = node.end_position().row;
            comment_lines += end - start + 1;
        }
    });

    let code_lines = total_non_blank.saturating_sub(comment_lines);
    (comment_lines, code_lines)
}

/// Walk all descendants of a node, calling `f` on each.
fn walk_all<F: FnMut(Node)>(node: Node, cursor: &mut tree_sitter::TreeCursor, f: &mut F) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        f(current);
        let mut child_cursor = current.walk();
        for child in current.children(&mut child_cursor) {
            stack.push(child);
        }
    }
    // Keep cursor alive for borrow checker
    let _ = cursor;
}

// ── Code-style helpers ─────────────────────────────────────────────

/// Count top-level nodes matching any of the given kinds.
pub fn count_nodes_of_kind(root: Node, kinds: &[&str]) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            count += 1;
        }
    }
    count
}

/// Count named children of a parameters/parameter_list node.
pub fn count_parameters(params_node: Node) -> usize {
    let mut count = 0;
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        // Skip `self` parameter in Rust, receiver in Go, etc.
        let kind = child.kind();
        if kind == "self_parameter" || kind == "variadic_parameter" {
            continue;
        }
        count += 1;
    }
    count
}

/// Walk direct children of a block; after a node whose kind is in `return_kinds`,
/// collect positions of subsequent siblings. Returns vec of (line, column) 1-indexed.
pub fn find_unreachable_after(body: Node, return_kinds: &[&str]) -> Vec<(u32, u32)> {
    let mut results = Vec::new();

    // Some grammars (e.g., Go) wrap statements in a `statement_list` child.
    // If the body has such a wrapper, use its children instead.
    let effective_body = {
        let mut cursor = body.walk();
        let mut found = None;
        for child in body.named_children(&mut cursor) {
            if child.kind() == "statement_list" || child.kind() == "statement_block" {
                found = Some(child);
                break;
            }
        }
        found.unwrap_or(body)
    };

    let mut cursor = effective_body.walk();
    let children: Vec<_> = effective_body.children(&mut cursor).collect();

    let mut i = 0;
    while i < children.len() {
        let child = children[i];
        if is_return_like(child, return_kinds) {
            // Everything after this in the same block is unreachable
            for unreachable in &children[i + 1..] {
                // Skip closing braces and whitespace-only nodes
                let kind = unreachable.kind();
                if kind == "}"
                    || kind == "{"
                    || kind == "comment"
                    || kind == "line_comment"
                    || kind == "block_comment"
                {
                    continue;
                }
                let pos = unreachable.start_position();
                results.push((pos.row as u32 + 1, pos.column as u32 + 1));
            }
            break;
        }
        i += 1;
    }

    results
}

/// Check if a node is a return-like statement, either directly or wrapped
/// in an expression_statement (e.g., Rust `return;` becomes `expression_statement > return_expression`).
fn is_return_like(node: Node, return_kinds: &[&str]) -> bool {
    let kind = node.kind();
    if return_kinds.contains(&kind) {
        return true;
    }
    // Check for expression_statement or similar wrappers containing a return-like node
    if (kind == "expression_statement" || kind == "labeled_statement")
        && let Some(first_named) = node.named_child(0)
        && return_kinds.contains(&first_named.kind())
    {
        return true;
    }
    false
}

/// Compute a structural hash of a code block with identifiers normalized.
/// Identifiers are replaced by positional placeholders so structurally identical
/// blocks with different names produce the same hash.
pub fn hash_block_normalized(node: Node, source: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut counter = 0u32;
    hash_node_recursive(node, source, &mut hasher, &mut counter);
    hasher.finish()
}

fn hash_node_recursive(
    root: Node,
    source: &[u8],
    hasher: &mut std::collections::hash_map::DefaultHasher,
    counter: &mut u32,
) {
    // Stack-based DFS; children pushed in reverse for left-to-right hash ordering
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        kind.hash(hasher);

        if kind == "identifier"
            || kind == "field_identifier"
            || kind == "type_identifier"
            || kind == "property_identifier"
            || kind == "shorthand_property_identifier"
            || kind == "name"
        {
            counter.hash(hasher);
            *counter += 1;
        } else if node.child_count() == 0 {
            let text = node.utf8_text(source).unwrap_or("");
            text.hash(hasher);
        }

        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

/// Build a map of identifier/field_identifier name -> occurrence count across the entire tree.
/// Single O(n) pass. Used by dead_code pipelines to avoid O(n*m) per-function walks.
pub fn count_all_identifier_occurrences(root: Node, source: &[u8]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        let kind = current.kind();
        if (kind == "identifier" || kind == "field_identifier")
            && let Ok(text) = current.utf8_text(source)
            && !text.is_empty()
        {
            *counts.entry(text.to_string()).or_insert(0) += 1;
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    counts
}

/// Collect all identifier text within a subtree using stack-based iteration.
pub fn collect_identifiers(root: Node, source: &[u8]) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if matches!(
            kind,
            "identifier"
                | "field_identifier"
                | "type_identifier"
                | "property_identifier"
                | "shorthand_property_identifier"
                | "name"
                | "self"
                | "this"
                | "variable_name"
        ) && let Ok(text) = node.utf8_text(source)
        {
            ids.insert(text.to_string());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    ids
}

/// Find duplicate function bodies within a file.
/// Returns groups of (function_name, line, column) for functions with identical normalized bodies.
/// Only considers functions with bodies >= `min_lines` lines.
pub fn find_duplicate_bodies(
    root: Node,
    source: &[u8],
    func_kinds: &[&str],
    body_field: &str,
    name_field: &str,
    min_lines: usize,
) -> Vec<Vec<(String, u32, u32)>> {
    let mut hash_map: HashMap<u64, Vec<(String, u32, u32)>> = HashMap::new();

    collect_functions_iterative(
        root,
        source,
        func_kinds,
        body_field,
        name_field,
        min_lines,
        &mut hash_map,
    );

    hash_map
        .into_values()
        .filter(|group| group.len() >= 2)
        .collect()
}

fn collect_functions_iterative(
    root: Node,
    source: &[u8],
    func_kinds: &[&str],
    body_field: &str,
    name_field: &str,
    min_lines: usize,
    hash_map: &mut HashMap<u64, Vec<(String, u32, u32)>>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if func_kinds.contains(&node.kind())
            && let Some(body) = node.child_by_field_name(body_field)
        {
            let body_lines = body
                .end_position()
                .row
                .saturating_sub(body.start_position().row)
                + 1;
            if body_lines >= min_lines {
                let hash = hash_block_normalized(body, source);
                let name = node
                    .child_by_field_name(name_field)
                    .map(|n| n.utf8_text(source).unwrap_or("<unknown>").to_string())
                    .unwrap_or_else(|| "<anonymous>".to_string());
                let pos = node.start_position();
                hash_map.entry(hash).or_default().push((
                    name,
                    pos.row as u32 + 1,
                    pos.column as u32 + 1,
                ));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Find duplicate switch/match arms within switch/match statements.
/// Returns vec of (match_stmt_line, duplicate_arm_lines).
pub fn find_duplicate_arms(
    root: Node,
    source: &[u8],
    switch_kind: &str,
    arm_kind: &str,
    body_field: Option<&str>,
) -> Vec<(u32, Vec<u32>)> {
    let mut results = Vec::new();
    find_switches_iterative(
        root,
        source,
        switch_kind,
        arm_kind,
        body_field,
        &mut results,
    );
    results
}

fn find_switches_iterative(
    root: Node,
    source: &[u8],
    switch_kind: &str,
    arm_kind: &str,
    body_field: Option<&str>,
    results: &mut Vec<(u32, Vec<u32>)>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == switch_kind {
            let mut arm_hashes: Vec<(u64, u32)> = Vec::new();
            collect_arms_iterative(node, arm_kind, body_field, source, &mut arm_hashes);

            let mut seen: HashMap<u64, u32> = HashMap::new();
            let mut dup_lines = Vec::new();
            for (hash, line) in &arm_hashes {
                if seen.contains_key(hash) {
                    dup_lines.push(*line);
                } else {
                    seen.insert(*hash, *line);
                }
            }

            if !dup_lines.is_empty() {
                results.push((node.start_position().row as u32 + 1, dup_lines));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn collect_arms_iterative(
    root: Node,
    arm_kind: &str,
    body_field: Option<&str>,
    source: &[u8],
    arm_hashes: &mut Vec<(u64, u32)>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == arm_kind {
            if let Some(field) = body_field {
                if let Some(body) = node.child_by_field_name(field) {
                    let hash = hash_block_normalized(body, source);
                    arm_hashes.push((hash, node.start_position().row as u32 + 1));
                }
            } else {
                let hash = hash_arm_body_children(node, source);
                arm_hashes.push((hash, node.start_position().row as u32 + 1));
            }
            continue; // Don't descend into arms
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Hash the "body" portion of a switch/match arm by skipping the first named child
/// (the case value or pattern) and hashing everything else.
fn hash_arm_body_children(arm: Node, source: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut counter = 0u32;
    let mut cursor = arm.walk();
    let mut skipped_first = false;
    for child in arm.named_children(&mut cursor) {
        if !skipped_first {
            skipped_first = true;
            continue;
        }
        hash_node_recursive(child, source, &mut hasher, &mut counter);
    }
    hasher.finish()
}

/// Check if a method body references a specific identifier (e.g., "self", "this").
pub fn body_references_identifier(body: Node, source: &[u8], target: &str) -> bool {
    let ids = collect_identifiers(body, source);
    ids.contains(target)
}

// ── Architecture helpers ─────────────────────────────────────────────

/// Count top-level definitions matching any of the given node kinds.
/// This walks direct children of `root` (typically `source_file` or `program`).
pub fn count_top_level_definitions(root: Node, symbol_kinds: &[&str]) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if symbol_kinds.contains(&child.kind()) {
            count += 1;
        }
    }
    count
}

/// Count the depth (number of segments) in an import path.
/// `separator` is the path separator (e.g., "::" for Rust, "/" for JS, "." for Python).
pub fn count_path_depth(path: &str, separator: &str) -> usize {
    path.split(separator).filter(|s| !s.is_empty()).count()
}

/// Check if a file should be excluded from anemic module detection.
/// `file_path` is the relative path; `exclusions` are filename patterns (e.g., "main.rs", "index.ts").
pub fn is_entry_file(file_path: &str, exclusions: &[&str]) -> bool {
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    exclusions.contains(&file_name)
}

/// Check if a method body contains member access via a specific pattern (e.g., "self." in Python, "this." in Java).
/// Uses stack-based iteration with early termination.
pub fn body_has_member_access(
    body: Node,
    source: &[u8],
    access_kind: &str,
    object_text: &str,
) -> bool {
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if node.kind() == access_kind
            && let Some(obj) = node.child_by_field_name("object")
            && obj.utf8_text(source).unwrap_or("") == object_text
        {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

// ── False-positive suppression helpers ────────────────────────────────

/// Check if a file path indicates test code (language-agnostic).
pub fn is_test_file(file_path: &str) -> bool {
    let path = file_path.replace('\\', "/");
    // Directory patterns
    if path.contains("/tests/")
        || path.contains("/test/")
        || path.contains("/__tests__/")
        || path.contains("/testing/")
        || path.contains("/testdata/")
    {
        return true;
    }
    // File name patterns
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    // Rust: _test.rs, Go: _test.go
    if file_name.ends_with("_test.rs") || file_name.ends_with("_test.go") {
        return true;
    }
    // Python: test_*.py, *_test.py, conftest.py
    if (file_name.starts_with("test_") && file_name.ends_with(".py"))
        || (file_name.ends_with("_test.py"))
        || file_name == "conftest.py"
    {
        return true;
    }
    // Java: *Test.java, *Tests.java, *Spec.java
    if file_name.ends_with("Test.java")
        || file_name.ends_with("Tests.java")
        || file_name.ends_with("Spec.java")
    {
        return true;
    }
    // JS/TS: *.test.ts, *.spec.ts, *.test.js, *.spec.js, *.test.tsx, *.spec.tsx
    let lower = file_name.to_lowercase();
    if lower.contains(".test.") || lower.contains(".spec.") {
        return true;
    }
    false
}

/// Walk the parent chain of `node`; return true if any ancestor's kind is in `kinds`.
pub fn ancestor_has_kind(node: Node, kinds: &[&str]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if kinds.contains(&parent.kind()) {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Check if a node's text contains a given substring.
pub fn node_text_contains(node: Node, source: &[u8], text: &str) -> bool {
    node.utf8_text(source).unwrap_or("").contains(text)
}

// ── Rust-specific helpers ─────────────────────────────────────────────

/// Check if a node is inside a Rust test context (#[test] fn, #[cfg(test)] mod, mod tests).
pub fn is_test_context_rust(node: Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        let kind = n.kind();
        if kind == "function_item" {
            // Check if this function has a #[test] attribute
            if let Some(prev) = n.prev_named_sibling()
                && prev.kind() == "attribute_item"
            {
                let attr_text = prev.utf8_text(source).unwrap_or("");
                if attr_text.contains("test") {
                    return true;
                }
            }
        }
        if kind == "mod_item" {
            // Check for mod tests or #[cfg(test)]
            if let Some(name) = n.child_by_field_name("name")
                && name.utf8_text(source).unwrap_or("") == "tests"
            {
                return true;
            }
            if let Some(prev) = n.prev_named_sibling()
                && prev.kind() == "attribute_item"
            {
                let attr_text = prev.utf8_text(source).unwrap_or("");
                if attr_text.contains("cfg(test)") {
                    return true;
                }
            }
        }
        current = n.parent();
    }
    false
}

/// Check if a node or its ancestors have an attribute_item containing `attr` text.
pub fn has_attribute_text(node: Node, source: &[u8], attr: &str) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        // Check previous siblings for attribute_item
        let mut prev = n.prev_named_sibling();
        while let Some(p) = prev {
            if p.kind() == "attribute_item" {
                if p.utf8_text(source).unwrap_or("").contains(attr) {
                    return true;
                }
            } else {
                break;
            }
            prev = p.prev_named_sibling();
        }
        current = n.parent();
    }
    false
}

/// Check if a struct has a #[derive(Name)] attribute.
pub fn struct_has_derive(node: Node, source: &[u8], name: &str) -> bool {
    // Walk from the struct_item, check previous siblings for #[derive(...Name...)]
    let struct_node = if node.kind() == "struct_item" {
        node
    } else {
        // Walk up to find struct_item
        let mut current = node.parent();
        loop {
            match current {
                Some(n) if n.kind() == "struct_item" => break n,
                Some(n) => current = n.parent(),
                None => return false,
            }
        }
    };
    let mut prev = struct_node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() == "attribute_item" {
            let text = p.utf8_text(source).unwrap_or("");
            if text.contains("derive") && text.contains(name) {
                return true;
            }
        } else {
            break;
        }
        prev = p.prev_named_sibling();
    }
    false
}

/// Check if node is inside a spawn_blocking or block_in_place closure.
pub fn is_inside_spawn_blocking(node: Node, source: &[u8]) -> bool {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "call_expression" {
            let call_text = p.utf8_text(source).unwrap_or("");
            if call_text.contains("spawn_blocking") || call_text.contains("block_in_place") {
                return true;
            }
        }
        current = p.parent();
    }
    false
}

/// Check if an impl_item is a trait impl (impl Trait for Type).
pub fn is_trait_impl(impl_node: Node, source: &[u8]) -> bool {
    let text = impl_node.utf8_text(source).unwrap_or("");
    // A trait impl contains "for" between the trait name and type name
    // e.g., "impl Display for Foo { ... }"
    // Simple heuristic: check first line for pattern "impl ... for ..."
    if let Some(first_line) = text.lines().next() {
        let trimmed = first_line.trim();
        if trimmed.starts_with("impl") {
            // Check if there's a "for" keyword before the opening brace
            let before_brace = if let Some(idx) = trimmed.find('{') {
                &trimmed[..idx]
            } else {
                trimmed
            };
            // Split by whitespace and check for "for" token
            let tokens: Vec<&str> = before_brace.split_whitespace().collect();
            return tokens.contains(&"for");
        }
    }
    false
}

/// Check if a Rust function is named main.
pub fn is_main_function_rust(node: Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "function_item"
            && let Some(name) = n.child_by_field_name("name")
        {
            return name.utf8_text(source).unwrap_or("") == "main";
        }
        current = n.parent();
    }
    false
}

/// Check if a Rust method call has a chained .lock() in its receiver.
pub fn receiver_has_lock(call_node: Node, source: &[u8]) -> bool {
    // call_expression > field_expression > value (the receiver chain)
    if let Some(field_expr) = call_node.child_by_field_name("function")
        && let Some(value) = field_expr.child_by_field_name("value")
    {
        let receiver_text = value.utf8_text(source).unwrap_or("");
        return receiver_text.contains(".lock()");
    }
    false
}

// ── Go-specific helpers ───────────────────────────────────────────────

/// Check if a Go node is inside a test function (func Test*, func Benchmark*) or _test.go file.
pub fn is_test_context_go(node: Node, source: &[u8], file_path: &str) -> bool {
    if file_path.ends_with("_test.go") {
        return true;
    }
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "function_declaration"
            && let Some(name) = n.child_by_field_name("name")
        {
            let name_text = name.utf8_text(source).unwrap_or("");
            if name_text.starts_with("Test") || name_text.starts_with("Benchmark") {
                return true;
            }
        }
        current = n.parent();
    }
    false
}

/// Check if a Go node is inside func init() or func main().
pub fn is_init_or_main_go(node: Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "function_declaration"
            && let Some(name) = n.child_by_field_name("name")
        {
            let name_text = name.utf8_text(source).unwrap_or("");
            if name_text == "init" || name_text == "main" {
                return true;
            }
        }
        current = n.parent();
    }
    false
}

/// Check if a Go function name starts with "New" (constructor convention).
pub fn is_go_constructor(node: Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "function_declaration"
            && let Some(name) = n.child_by_field_name("name")
        {
            let name_text = name.utf8_text(source).unwrap_or("");
            if name_text.starts_with("New") || name_text.starts_with("Init") {
                return true;
            }
        }
        current = n.parent();
    }
    false
}

// ── Java-specific helpers ─────────────────────────────────────────────

/// Check if a Java node has a specific annotation (e.g., @Test, @Override).
pub fn has_annotation(node: Node, source: &[u8], name: &str) -> bool {
    // Java annotations are children of `modifiers` node, which is a sibling of the method
    // or they can be marker_annotation or annotation nodes
    let target = if node.kind() == "method_declaration"
        || node.kind() == "class_declaration"
        || node.kind() == "field_declaration"
    {
        node
    } else {
        // Walk up to find the enclosing declaration
        let mut current = node.parent();
        loop {
            match current {
                Some(n) if n.kind() == "method_declaration" || n.kind() == "class_declaration" => {
                    break n;
                }
                Some(n) => current = n.parent(),
                None => return false,
            }
        }
    };
    // Check for modifiers child containing the annotation
    let mut cursor = target.walk();
    for child in target.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner_cursor = child.walk();
            for modifier_child in child.children(&mut inner_cursor) {
                if modifier_child.kind() == "marker_annotation"
                    || modifier_child.kind() == "annotation"
                {
                    let text = modifier_child.utf8_text(source).unwrap_or("");
                    if text.contains(name) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if a Java node is inside a try-with-resources resource spec.
pub fn is_in_try_with_resources(node: Node) -> bool {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "try_with_resources_statement" || p.kind() == "resource_specification" {
            return true;
        }
        current = p.parent();
    }
    false
}

// ── JS/TS-specific helpers ────────────────────────────────────────────

/// Check if a JS/TS node is inside a test context (describe, it, test, beforeEach blocks).
pub fn is_test_context_js(node: Node, source: &[u8]) -> bool {
    let test_callee_names = [
        "describe",
        "it",
        "test",
        "beforeEach",
        "afterEach",
        "beforeAll",
        "afterAll",
    ];
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "call_expression" {
            // Check if the callee is one of the test framework functions
            if let Some(func) = p.child_by_field_name("function") {
                let func_text = func.utf8_text(source).unwrap_or("");
                if test_callee_names.contains(&func_text) {
                    return true;
                }
            }
        }
        current = p.parent();
    }
    false
}

// ── Python-specific helpers ───────────────────────────────────────────

/// Check if a Python node is inside a test context (def test_*, class Test*, conftest.py).
pub fn is_test_context_python(node: Node, source: &[u8], file_path: &str) -> bool {
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    if file_name == "conftest.py" {
        return true;
    }
    let mut current = Some(node);
    while let Some(n) = current {
        match n.kind() {
            "function_definition" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let name_text = name.utf8_text(source).unwrap_or("");
                    if name_text.starts_with("test_") {
                        return true;
                    }
                }
            }
            "class_definition" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let name_text = name.utf8_text(source).unwrap_or("");
                    if name_text.starts_with("Test") {
                        return true;
                    }
                }
            }
            _ => {}
        }
        current = n.parent();
    }
    false
}

/// Extract the receiver text from a method call's field_expression.
/// For `receiver.method()`, returns the text of `receiver`.
/// Handles multiple AST patterns:
/// - Rust/Go: `call_expression > function: field_expression > value`
/// - JS/TS: `call_expression > function: member_expression > object`
/// - Java/C#: `method_invocation > object`
/// - Python: `call > function: attribute > object`
/// - PHP: `member_call_expression > object`
pub fn extract_receiver_text<'a>(call_node: Node<'a>, source: &'a [u8]) -> &'a str {
    // Direct object field (Java method_invocation, PHP member_call_expression)
    if let Some(obj) = call_node.child_by_field_name("object") {
        return obj.utf8_text(source).unwrap_or("");
    }
    // Rust/Go/JS/TS: call_expression > function: (field_expression|member_expression)
    if let Some(func) = call_node.child_by_field_name("function")
        && (func.kind() == "field_expression"
            || func.kind() == "member_expression"
            || func.kind() == "attribute")
    {
        if let Some(obj) = func.child_by_field_name("object") {
            return obj.utf8_text(source).unwrap_or("");
        }
        if let Some(val) = func.child_by_field_name("value") {
            return val.utf8_text(source).unwrap_or("");
        }
    }
    ""
}

/// Check if receiver text matches any of the given patterns (case-insensitive contains).
pub fn receiver_matches_any(receiver: &str, patterns: &[&str]) -> bool {
    let lower = receiver.to_lowercase();
    patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js_config() -> ControlFlowConfig {
        ControlFlowConfig {
            decision_point_kinds: &[
                "if_statement",
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_case",
                "catch_clause",
            ],
            nesting_increments: &[
                "if_statement",
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_statement",
                "catch_clause",
            ],
            flat_increments: &["else_clause"],
            logical_operators: &["&&", "||"],
            binary_expression_kind: "binary_expression",
            ternary_kind: Some("ternary_expression"),
            comment_kinds: &["comment"],
        }
    }

    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn cyclomatic_simple_function() {
        let src = "function foo() { if (x) { } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        // function_declaration -> body is statement_block
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cc = compute_cyclomatic(body, &js_config(), src.as_bytes());
        assert_eq!(cc, 2); // 1 base + 1 if
    }

    #[test]
    fn cyclomatic_with_logical_ops() {
        let src = "function foo() { if (a && b || c) { } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cc = compute_cyclomatic(body, &js_config(), src.as_bytes());
        // 1 base + 1 if + 2 logical ops (&&, ||)
        assert_eq!(cc, 4);
    }

    #[test]
    fn cognitive_nested() {
        let src = "function foo() { if (x) { for (;;) { if (y) { } } } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let cog = compute_cognitive(body, &js_config(), src.as_bytes());
        // if: +1 (nesting=0), for: +1+1=2 (nesting=1), inner if: +1+2=3 (nesting=2)
        // total = 1 + 2 + 3 = 6
        assert_eq!(cog, 6);
    }

    #[test]
    fn function_lines_count() {
        let src = "function foo() {\n  let a = 1;\n  let b = 2;\n  return a + b;\n}";
        let tree = parse_js(src);
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        let (lines, _stmts) = count_function_lines(body);
        assert_eq!(lines, 5);
    }

    #[test]
    fn comment_ratio_basic() {
        let src = "// comment\nlet x = 1;\nlet y = 2;\n";
        let tree = parse_js(src);
        let (comment_lines, code_lines) =
            compute_comment_ratio(tree.root_node(), src.as_bytes(), &js_config());
        assert_eq!(comment_lines, 1);
        assert_eq!(code_lines, 2);
    }

    // ── Tests for false-positive suppression helpers ──────────────

    #[test]
    fn test_is_test_file() {
        assert!(is_test_file("src/foo_test.rs"));
        assert!(is_test_file("pkg/handler_test.go"));
        assert!(is_test_file("tests/test_main.py"));
        assert!(is_test_file("src/main.test.ts"));
        assert!(is_test_file("src/__tests__/foo.js"));
        assert!(is_test_file("UserTest.java"));
        assert!(!is_test_file("src/main.rs"));
        assert!(!is_test_file("src/handler.go"));
    }

    #[test]
    fn test_ancestor_has_kind() {
        let src = "function foo() { if (x) { let y = 1; } }";
        let tree = parse_js(src);
        let root = tree.root_node();
        // Navigate to the number literal inside the if
        let func = root.named_child(0).unwrap();
        let body = func.child_by_field_name("body").unwrap();
        // Find the if_statement
        let mut cursor = body.walk();
        let if_stmt = body
            .named_children(&mut cursor)
            .find(|c| c.kind() == "if_statement")
            .unwrap();
        assert!(ancestor_has_kind(if_stmt, &["statement_block"]));
        assert!(!ancestor_has_kind(if_stmt, &["for_statement"]));
    }

    #[test]
    fn test_receiver_matches_any() {
        assert!(receiver_matches_any("dbConn", &["db", "conn"]));
        assert!(receiver_matches_any("myPool", &["pool"]));
        assert!(!receiver_matches_any("myList", &["db", "conn"]));
    }
}
