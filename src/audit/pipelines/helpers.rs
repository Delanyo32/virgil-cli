use std::collections::HashMap;
use tree_sitter::Node;

use crate::graph::CodeGraph;

// ── False-positive suppression helpers ────────────────────────────────

/// Check if a file path indicates test code (language-agnostic).
pub fn is_test_file(file_path: &str) -> bool {
    // File name patterns first — no allocation needed, short-circuits the common cases.
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
        || file_name.ends_with("_test.py")
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
    // C#: *Tests.cs, *Test.cs, *Spec.cs
    if file_name.ends_with("Tests.cs")
        || file_name.ends_with("Test.cs")
        || file_name.ends_with("Spec.cs")
    {
        return true;
    }
    // PHP: *Test.php (PHPUnit convention)
    if file_name.ends_with("Test.php") {
        return true;
    }
    // C++: *_test.cpp / *_test.cc (GoogleTest / Catch2 suffix)
    if file_name.ends_with("_test.cpp")
        || file_name.ends_with("_test.cc")
        || file_name.ends_with("_unittest.cpp")
    {
        return true;
    }
    // C++: *Test.cpp (CppUnit / GoogleTest class-name convention) — length guard avoids
    // matching a file literally named "Test.cpp"
    if file_name.ends_with("Test.cpp") && file_name.len() > "Test.cpp".len() {
        return true;
    }
    // C++: test_*.cpp / test_*.cc (prefix pattern)
    if (file_name.starts_with("test_") && file_name.ends_with(".cpp"))
        || (file_name.starts_with("test_") && file_name.ends_with(".cc"))
    {
        return true;
    }
    // JS/TS: *.test.ts, *.spec.ts, *.test.js, *.spec.js, *.test.tsx, *.spec.tsx
    let lower = file_name.to_lowercase();
    if lower.contains(".test.") || lower.contains(".spec.") {
        return true;
    }
    // Directory patterns — normalize separators only if filename checks didn't match.
    let path = file_path.replace('\\', "/");
    if path.contains("/tests/")
        || path.starts_with("tests/")
        || path.contains("/test/")
        || path.starts_with("test/")
        || path.contains("/__tests__/")
        || path.starts_with("__tests__/")
        || path.contains("/testing/")
        || path.starts_with("testing/")
        || path.contains("/testdata/")
        || path.starts_with("testdata/")
    {
        return true;
    }
    false
}

/// Returns `true` if the file should be excluded from cross-file architecture
/// analysis. Covers test files, generated files (path-detectable), and
/// vendor/third-party directories. Does not require file source bytes.
pub fn is_excluded_for_arch_analysis(path: &str) -> bool {
    if is_test_file(path) {
        return true;
    }
    let p = path.replace('\\', "/");
    // Generated file patterns detectable from path alone
    if p.ends_with(".pb.go")
        || p.ends_with("_gen.go")
        || p.ends_with("_generated.go")
        || p.ends_with(".pb.h")
        || p.ends_with(".pb.cc")
        || p.contains("/generated/")
        || p.starts_with("generated/")
    {
        return true;
    }
    // Vendor / third-party / build directories
    if p.contains("/vendor/")
        || p.starts_with("vendor/")
        || p.contains("/third_party/")
        || p.starts_with("third_party/")
        || p.contains("/node_modules/")
        || p.starts_with("node_modules/")
        || p.contains("/_deps/")
        || p.starts_with("_deps/")
    {
        return true;
    }
    false
}

/// Returns `true` if the file is a barrel / re-export aggregator by name.
/// Barrel files (index.ts, __init__.py, mod.rs, etc.) should not count as
/// a depth hop in dependency chains and should not trigger efferent coupling
/// findings, because their high import count is intentional.
pub fn is_barrel_file(path: &str) -> bool {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    matches!(
        file_name,
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "__init__.py" | "mod.rs"
    )
}

// ── Dead-code helpers ─────────────────────────────────────────────────

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

// ── Literal / taint-pipeline helpers ─────────────────────────────────
// Used by taint-based SQL injection pipelines (permanent Rust exceptions).

/// Check if a Go AST node is a safe literal value.
pub fn is_literal_node_go(node: tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "interpreted_string_literal"
            | "raw_string_literal"
            | "int_literal"
            | "float_literal"
            | "true"
            | "false"
            | "nil"
    )
}

/// Check if a Java AST node is a safe literal value.
pub fn is_literal_node_java(node: tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "string_literal"
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "character_literal"
            | "true"
            | "false"
            | "null_literal"
    )
}

/// Check if a C# AST node is a safe literal value.
/// Note: `interpolated_string_expression` is NOT safe (contains dynamic parts).
pub fn is_literal_node_csharp(node: tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "string_literal"
            | "verbatim_string_literal"
            | "integer_literal"
            | "real_literal"
            | "boolean_literal"
            | "null_literal"
            | "character_literal"
    )
}

/// Check if an expression node is safe (literal, or binary expression of literals).
/// Recurses into `binary_expression` and `concatenated_string` nodes.
pub fn is_safe_expression(
    node: tree_sitter::Node,
    is_literal: impl Fn(tree_sitter::Node) -> bool + Copy,
) -> bool {
    if is_literal(node) {
        return true;
    }
    // Parenthesized expression — unwrap
    if node.kind() == "parenthesized_expression"
        && let Some(inner) = node.named_child(0)
    {
        return is_safe_expression(inner, is_literal);
    }
    // Binary expression (string concatenation with +, etc.)
    if node.kind() == "binary_expression" {
        let mut cursor = node.walk();
        let mut has_children = false;
        for child in node.named_children(&mut cursor) {
            has_children = true;
            if !is_safe_expression(child, is_literal) {
                return false;
            }
        }
        return has_children;
    }
    false
}

/// Check if ALL named children of an argument list node are safe expressions.
pub fn all_args_are_literals(
    args_node: tree_sitter::Node,
    is_literal: impl Fn(tree_sitter::Node) -> bool + Copy,
) -> bool {
    let mut cursor = args_node.walk();
    let mut count = 0;
    for child in args_node.named_children(&mut cursor) {
        count += 1;
        if !is_safe_expression(child, is_literal) {
            return false;
        }
    }
    count > 0
}

/// Find the enclosing function for a given line and count its direct callers in the graph.
///
/// Returns `Some((node_index, caller_count))` for the narrowest enclosing `Symbol` node,
/// or `None` if no enclosing symbol is found.
pub fn find_enclosing_function_callers(
    graph: &CodeGraph,
    file_path: &str,
    line: u32,
) -> Option<(petgraph::graph::NodeIndex, usize)> {
    use crate::graph::NodeWeight;
    let mut best_idx = None;
    let mut best_range = u32::MAX;

    for idx in graph.graph.node_indices() {
        if let NodeWeight::Symbol {
            file_path: fp,
            start_line,
            end_line,
            ..
        } = &graph.graph[idx]
        {
            if fp == file_path && *start_line <= line && line <= *end_line {
                let range = end_line - start_line;
                if range < best_range {
                    best_range = range;
                    best_idx = Some(idx);
                }
            }
        }
    }

    best_idx.map(|idx| {
        let callers = graph.traverse_callers(&[idx], 1);
        (idx, callers.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_is_excluded_for_arch_analysis() {
        // Test files are excluded
        assert!(is_excluded_for_arch_analysis("src/foo_test.rs"));
        assert!(is_excluded_for_arch_analysis("pkg/handler_test.go"));
        assert!(is_excluded_for_arch_analysis("tests/integration.rs"));
        // Generated file patterns
        assert!(is_excluded_for_arch_analysis("proto/service.pb.go"));
        assert!(is_excluded_for_arch_analysis("models/user_gen.go"));
        assert!(is_excluded_for_arch_analysis("models/schema_generated.go"));
        assert!(is_excluded_for_arch_analysis("include/api.pb.h"));
        assert!(is_excluded_for_arch_analysis("src/generated/schema.ts"));
        assert!(is_excluded_for_arch_analysis("generated/models.rs"));
        // Vendor / third-party directories
        assert!(is_excluded_for_arch_analysis("vendor/serde/src/lib.rs"));
        assert!(is_excluded_for_arch_analysis("third_party/openssl/ssl.h"));
        assert!(is_excluded_for_arch_analysis("node_modules/react/index.js"));
        assert!(is_excluded_for_arch_analysis("_deps/googletest/src/gtest.cc"));
        // Normal source files are NOT excluded
        assert!(!is_excluded_for_arch_analysis("src/main.rs"));
        assert!(!is_excluded_for_arch_analysis("src/auth/service.ts"));
        assert!(!is_excluded_for_arch_analysis("lib/utils.py"));
    }

    #[test]
    fn test_is_barrel_file() {
        assert!(is_barrel_file("src/index.ts"));
        assert!(is_barrel_file("components/index.tsx"));
        assert!(is_barrel_file("src/index.js"));
        assert!(is_barrel_file("lib/index.jsx"));
        assert!(is_barrel_file("src/models/__init__.py"));
        assert!(is_barrel_file("src/models/mod.rs"));
        // Non-barrel files
        assert!(!is_barrel_file("src/auth.ts"));
        assert!(!is_barrel_file("src/service/user.ts"));
        assert!(!is_barrel_file("src/models/user.rs"));
        assert!(!is_barrel_file("src/reindex.ts")); // contains "index" but not a barrel
    }

    #[test]
    fn test_is_test_file_cpp() {
        // Suffix patterns
        assert!(is_test_file("network_handler_test.cpp"));
        assert!(is_test_file("network_handler_test.cc"));
        assert!(is_test_file("network_handler_unittest.cpp"));
        assert!(is_test_file("NetworkHandlerTest.cpp")); // *Test.cpp
        // Prefix patterns
        assert!(is_test_file("test_network_handler.cpp"));
        assert!(is_test_file("test_network_handler.cc"));
        // Already covered by .test. — ensure still passes
        assert!(is_test_file("network.test.cpp"));
        // Negative — "test" as substring is not enough
        assert!(!is_test_file("attestation.cpp"));
        assert!(!is_test_file("latest_data.cpp"));
    }
}
