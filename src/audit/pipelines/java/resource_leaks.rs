use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::has_suppress_warnings;

use super::primitives::{
    compile_local_var_decl_query, extract_snippet, find_capture_index, node_text,
};

const RESOURCE_TYPES: &[&str] = &[
    "Connection",
    "Statement",
    "PreparedStatement",
    "ResultSet",
    "InputStream",
    "OutputStream",
    "FileInputStream",
    "FileOutputStream",
    "BufferedReader",
    "BufferedWriter",
    "FileReader",
    "FileWriter",
    "Socket",
    "ServerSocket",
    "Scanner",
    "PrintWriter",
    "Channel",
    "HttpClient",
    "CloseableHttpResponse",
    "RandomAccessFile",
    "ObjectInputStream",
    "ObjectOutputStream",
    "ZipInputStream",
    "ZipOutputStream",
    "EntityManager",
    "Session",
    "DatagramSocket",
];

/// Factory method patterns: (receiver, method_name)
const FACTORY_PATTERNS: &[(&str, &str)] = &[
    ("DriverManager", "getConnection"),
    ("Files", "newInputStream"),
    ("Files", "newOutputStream"),
    ("Files", "newBufferedReader"),
    ("Files", "newBufferedWriter"),
    ("DataSource", "getConnection"),
];

pub struct ResourceLeaksPipeline {
    local_var_query: Arc<Query>,
    resource_types: HashSet<&'static str>,
}

impl ResourceLeaksPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            local_var_query: compile_local_var_decl_query()?,
            resource_types: RESOURCE_TYPES.iter().copied().collect(),
        })
    }
}

/// Return graduated severity based on the resource type.
fn severity_for_type(type_text: &str) -> &'static str {
    match type_text {
        "Connection" | "Statement" | "PreparedStatement" | "ResultSet" => "error",
        "InputStream" | "OutputStream" | "FileInputStream" | "FileOutputStream"
        | "BufferedReader" | "BufferedWriter" | "FileReader" | "FileWriter"
        | "ObjectInputStream" | "ObjectOutputStream" | "ZipInputStream" | "ZipOutputStream"
        | "RandomAccessFile" => "warning",
        _ => "info",
    }
}

impl GraphPipeline for ResourceLeaksPipeline {
    fn name(&self) -> &str {
        "resource_leaks"
    }

    fn description(&self) -> &str {
        "Detects resource types created outside try-with-resources — potential resource leak"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.local_var_query, tree.root_node(), source);

        let var_type_idx = find_capture_index(&self.local_var_query, "var_type");
        let var_name_idx = find_capture_index(&self.local_var_query, "var_name");
        let creation_idx = find_capture_index(&self.local_var_query, "creation");
        let var_decl_idx = find_capture_index(&self.local_var_query, "var_decl");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_type_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_name_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_decl_idx)
                .map(|c| c.node);

            // Must have object_creation_expression (new ...)
            let Some(creation) = creation_node else {
                continue;
            };

            // Skip if the creation is wrapped inside another new expression (wrapping pattern)
            // e.g., BufferedReader br = new BufferedReader(new FileReader("f"))
            // The inner `new FileReader("f")` is passed as an arg to the outer `new BufferedReader(...)`.
            if is_wrapped_creation(creation) {
                continue;
            }

            if let (Some(type_node), Some(name_node), Some(decl_node)) =
                (type_node, name_node, decl_node)
            {
                // Extract the base type name (handle generic_type by getting its first child)
                let type_text = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| node_text(n, source))
                        .unwrap_or("")
                } else {
                    node_text(type_node, source)
                };

                if !self.resource_types.contains(type_text) {
                    continue;
                }

                // Check if inside try-with-resources resource_specification
                if is_in_try_with_resources(decl_node) {
                    continue;
                }

                // Check for @SuppressWarnings("resource")
                if has_suppress_warnings(decl_node, source, "resource") {
                    continue;
                }

                let var_name = node_text(name_node, source);

                // Check if closed in a finally block
                if has_finally_close(decl_node, source, var_name) {
                    continue;
                }

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity_for_type(type_text).to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "resource_leak".to_string(),
                    message: format!(
                        "`{type_text} {var_name}` is created outside try-with-resources — potential resource leak"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        // Second pass: detect factory method resources
        drop(matches);
        let mut cursor2 = QueryCursor::new();
        let mut matches2 = cursor2.matches(&self.local_var_query, tree.root_node(), source);

        while let Some(m) = matches2.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_type_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_name_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_decl_idx)
                .map(|c| c.node);

            // Skip if there IS an object_creation_expression (already handled above)
            if creation_node.is_some() {
                continue;
            }

            if let (Some(type_node), Some(name_node), Some(decl_node)) =
                (type_node, name_node, decl_node)
            {
                // Extract the base type name
                let type_text = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| node_text(n, source))
                        .unwrap_or("")
                } else {
                    node_text(type_node, source)
                };

                if !self.resource_types.contains(type_text) {
                    continue;
                }

                // Look for the variable_declarator to check its value
                let declarator = {
                    let mut found = None;
                    let mut walk = decl_node.walk();
                    for child in decl_node.children(&mut walk) {
                        if child.kind() == "variable_declarator" {
                            found = Some(child);
                            break;
                        }
                    }
                    found
                };

                let Some(declarator) = declarator else {
                    continue;
                };

                // Check if the value is a method_invocation matching factory patterns
                let value_node = declarator.child_by_field_name("value");
                let Some(value) = value_node else {
                    continue;
                };

                if value.kind() != "method_invocation" {
                    continue;
                }

                if !is_factory_method(value, source) {
                    continue;
                }

                // Apply the same checks as the first pass
                if is_in_try_with_resources(decl_node) {
                    continue;
                }

                if has_suppress_warnings(decl_node, source, "resource") {
                    continue;
                }

                let var_name = node_text(name_node, source);

                if has_finally_close(decl_node, source, var_name) {
                    continue;
                }

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity_for_type(type_text).to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "resource_leak".to_string(),
                    message: format!(
                        "`{type_text} {var_name}` is created via factory method outside try-with-resources — potential resource leak"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

/// Check if a method_invocation matches known factory patterns like `DriverManager.getConnection(...)`.
fn is_factory_method(method_invocation: tree_sitter::Node, source: &[u8]) -> bool {
    // method_invocation has children: object (identifier or field_access), ".", name (identifier), argument_list
    // Try to extract receiver.method pattern
    let method_name_node = method_invocation.child_by_field_name("name");
    let object_node = method_invocation.child_by_field_name("object");

    if let (Some(obj), Some(name)) = (object_node, method_name_node) {
        let receiver = node_text(obj, source);
        let method = node_text(name, source);
        for &(pat_receiver, pat_method) in FACTORY_PATTERNS {
            if receiver == pat_receiver && method == pat_method {
                return true;
            }
        }
    }
    false
}

/// Check if an object_creation_expression node is wrapped inside another new expression.
/// This handles patterns like: `new BufferedReader(new FileReader("file"))`.
/// The inner `new FileReader(...)` is an argument to the outer constructor, so it is being
/// wrapped and will be closed when the outer resource is closed.
fn is_wrapped_creation(creation_node: tree_sitter::Node) -> bool {
    if let Some(parent) = creation_node.parent() {
        // Direct parent is another object_creation_expression
        if parent.kind() == "object_creation_expression" {
            return true;
        }
        // Parent is argument_list whose parent is object_creation_expression
        if parent.kind() == "argument_list"
            && let Some(grandparent) = parent.parent()
            && grandparent.kind() == "object_creation_expression"
        {
            return true;
        }
    }
    false
}

/// Check if a node is inside the `resource_specification` of a `try_with_resources_statement`.
/// Walk parents: if we encounter `resource_specification` before `try_with_resources_statement`,
/// the node is in the resources (safe). If we hit `try_with_resources_statement` first without
/// passing through `resource_specification`, the node is in the body (not safe).
fn is_in_try_with_resources(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "resource_specification" {
            // We passed through resource_specification — this is a resource declaration
            return true;
        }
        if p.kind() == "try_with_resources_statement" {
            // Hit try-with-resources without going through resource_specification — in body
            return false;
        }
        if p.kind() == "method_declaration"
            || p.kind() == "constructor_declaration"
            || p.kind() == "class_declaration"
        {
            return false;
        }
        parent = p.parent();
    }
    false
}

/// Check if there is a `finally` block that closes the variable.
/// Two strategies:
/// 1. Walk up from the declaration: if the declaration is inside a try statement that has
///    a finally clause closing the variable, return true.
/// 2. Look at sibling nodes: if the declaration's parent block contains a try statement
///    (after the declaration) with a finally clause closing the variable, return true.
///    This handles the common pattern: `Connection c = new Connection(); try { ... } finally { c.close(); }`
fn has_finally_close(node: tree_sitter::Node, source: &[u8], var_name: &str) -> bool {
    // Strategy 1: walk up to find enclosing try statements with finally
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "try_statement" || p.kind() == "try_with_resources_statement" {
            let mut walk = p.walk();
            for child in p.children(&mut walk) {
                if child.kind() == "finally_clause"
                    && finally_contains_close(child, source, var_name)
                {
                    return true;
                }
            }
        }
        if p.kind() == "method_declaration"
            || p.kind() == "constructor_declaration"
            || p.kind() == "class_declaration"
        {
            break;
        }
        current = p.parent();
    }

    // Strategy 2: look at sibling try statements in the same block
    let decl_end = node.end_byte();
    if let Some(parent) = node.parent()
        && (parent.kind() == "block" || parent.kind() == "method_body")
    {
        let mut walk = parent.walk();
        for child in parent.children(&mut walk) {
            // Only look at try statements that appear after the declaration
            if child.start_byte() <= decl_end {
                continue;
            }
            if child.kind() == "try_statement" || child.kind() == "try_with_resources_statement"
            {
                let mut inner_walk = child.walk();
                for inner_child in child.children(&mut inner_walk) {
                    if inner_child.kind() == "finally_clause"
                        && finally_contains_close(inner_child, source, var_name)
                    {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Check if a finally_clause contains a `var_name.close()` call.
fn finally_contains_close(finally_node: tree_sitter::Node, source: &[u8], var_name: &str) -> bool {
    // Recursively walk the finally body looking for method_invocation nodes
    let mut stack = vec![finally_node];
    while let Some(node) = stack.pop() {
        if node.kind() == "method_invocation" {
            let object = node.child_by_field_name("object");
            let name = node.child_by_field_name("name");
            if let (Some(obj), Some(method_name)) = (object, name)
                && node_text(obj, source) == var_name && node_text(method_name, source) == "close"
            {
                return true;
            }
        }
        let mut walk = node.walk();
        for child in node.children(&mut walk) {
            stack.push(child);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ResourceLeaksPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
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
    fn detects_resource_leak() {
        let src = r#"
class Foo {
    void m() {
        Connection conn = new Connection();
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "resource_leak");
        assert!(findings[0].message.contains("Connection"));
    }

    #[test]
    fn clean_try_with_resources() {
        let src = r#"
class Foo {
    void m() {
        try (Connection conn = new Connection()) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_resource_type() {
        let src = r#"
class Foo {
    void m() {
        String s = new String("hello");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_input_stream_leak() {
        let src = r#"
class Foo {
    void m() {
        FileInputStream fis = new FileInputStream("file.txt");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FileInputStream"));
    }

    #[test]
    fn test_finally_close_is_clean() {
        let src = r#"
class Foo {
    void m() {
        Connection c = new Connection();
        try {
            c.execute();
        } finally {
            c.close();
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_try_body_resource_not_skipped() {
        let src = r#"
class Foo {
    void m() {
        try (Connection a = new Connection()) {
            FileInputStream b = new FileInputStream("x");
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FileInputStream"));
    }

    #[test]
    fn test_suppress_warnings() {
        let src = r#"
class Foo {
    @SuppressWarnings("resource")
    void m() {
        Connection c = new Connection();
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_severity_graduation() {
        let src = r#"
class Foo {
    void m() {
        Connection c = new Connection();
        Scanner s = new Scanner(System.in);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let conn = findings.iter().find(|f| f.message.contains("Connection")).unwrap();
        let scan = findings.iter().find(|f| f.message.contains("Scanner")).unwrap();
        assert_eq!(conn.severity, "error");
        assert_eq!(scan.severity, "info");
    }
}
