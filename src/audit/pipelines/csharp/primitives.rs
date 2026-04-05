use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::audit::pipelines::helpers::is_nolint_suppressed;
use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

/// Check if a node has a specific modifier by inspecting its flat `modifier` children.
/// C# uses direct `modifier` children (not a `modifiers` wrapper like Java).
pub fn has_modifier(node: tree_sitter::Node, source: &[u8], modifier_text: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" && child.utf8_text(source).unwrap_or("") == modifier_text {
            return true;
        }
    }
    false
}

pub fn compile_class_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(class_declaration
  name: (identifier) @class_name
  body: (declaration_list) @class_body) @class_decl
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile class_declaration query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_method_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(method_declaration
  returns: (_) @return_type
  name: (identifier) @method_name
  parameters: (parameter_list) @params) @method_decl
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile method_declaration query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_field_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_declaration
  (variable_declaration
    (variable_declarator
      (identifier) @field_name))) @field_decl
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile field_declaration query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_property_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(property_declaration
  type: (_) @prop_type
  name: (identifier) @prop_name) @prop_decl
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile property_declaration query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_catch_clause_query() -> Result<Arc<Query>> {
    let query_str = r#"
(catch_clause
  body: (block) @catch_body) @catch
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile catch_clause query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_return_null_query() -> Result<Arc<Query>> {
    let query_str = r#"
(return_statement (null_literal) @null_val) @return_stmt
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile return_null query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_invocation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(invocation_expression
  function: (_) @fn_expr
  arguments: (argument_list) @args) @invocation
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile invocation_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_member_access_query() -> Result<Arc<Query>> {
    let query_str = r#"
(member_access_expression
  name: (identifier) @member_name) @member_access
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile member_access_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_local_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(local_declaration_statement
  (variable_declaration
    type: (_) @var_type
    (variable_declarator
      name: (identifier) @var_name))) @var_decl
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile local_declaration_statement query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_parameter_query() -> Result<Arc<Query>> {
    let query_str = r#"
(parameter
  type: (_) @param_type
  name: (identifier) @param_name) @param
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile parameter query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_string_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
(string_literal) @str_lit
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile string_literal query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_object_creation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(object_creation_expression
  type: (_) @type_name
  arguments: (argument_list) @args) @creation
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile object_creation_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression
  left: (_) @left
  right: (_) @right) @bin_expr
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile binary_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_assignment_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_expression
  left: (_) @lhs
  right: (_) @rhs) @assign
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile assignment_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_interpolated_string_query() -> Result<Arc<Query>> {
    let query_str = r#"
(interpolated_string_expression) @interp_str
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile interpolated_string_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_method_with_body_query() -> Result<Arc<Query>> {
    let query_str = r#"
[
  (method_declaration
    name: (identifier) @method_name
    body: (block) @method_body) @method
  (constructor_declaration
    name: (identifier) @method_name
    body: (block) @method_body) @method
]
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile method_with_body query for C#")?;
    Ok(Arc::new(query))
}

// ── C#-specific suppression and context helpers ──────────────────────

/// Check if a node is suppressed by `#pragma warning disable` on the same or preceding lines.
/// Matches both blanket `#pragma warning disable` and targeted `#pragma warning disable CS1234`.
pub fn is_pragma_suppressed(source: &[u8], node: tree_sitter::Node, codes: &[&str]) -> bool {
    let source_str = match std::str::from_utf8(source) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let lines: Vec<&str> = source_str.lines().collect();
    let row = node.start_position().row;

    // Scan the current line and up to 2 preceding lines for pragma directives
    for offset in 0..=2 {
        if row < offset {
            continue;
        }
        let line_idx = row - offset;
        if line_idx >= lines.len() {
            continue;
        }
        let line = lines[line_idx].trim();
        if let Some(rest) = line.strip_prefix("#pragma warning disable") {
            let rest = rest.trim();
            if rest.is_empty() {
                // Blanket pragma — suppresses everything
                return true;
            }
            if codes.is_empty() {
                // No specific codes requested, any pragma counts
                return true;
            }
            // Check if any of the requested codes appear in the pragma
            for code in codes {
                if rest.split(',').any(|c| c.trim() == *code) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a C# declaration node has a specific attribute (e.g. `[SuppressMessage]`, `[ThreadStatic]`).
/// Walks the node's children looking for `attribute_list` containing the attribute name.
pub fn has_csharp_attribute(
    node: tree_sitter::Node,
    source: &[u8],
    attr_name: &str,
) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            let text = child.utf8_text(source).unwrap_or("");
            // Match [AttrName] or [AttrName(...)] — check the attribute name appears
            // after '[' and before '(' or ']'
            let mut inner_cursor = child.walk();
            for attr_child in child.children(&mut inner_cursor) {
                if attr_child.kind() == "attribute" {
                    if let Some(name_node) = attr_child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source).unwrap_or("");
                        if name == attr_name || name.ends_with(&format!("::{attr_name}")) {
                            return true;
                        }
                    }
                }
            }
            // Fallback: text-based check for cases where tree-sitter structure varies
            if text.contains(attr_name) {
                return true;
            }
        }
    }
    false
}

/// Combined C# suppression check: NOLINT comment, #pragma warning disable, or [SuppressMessage].
pub fn is_csharp_suppressed(
    source: &[u8],
    node: tree_sitter::Node,
    pipeline_name: &str,
) -> bool {
    // Check // NOLINT or // NOLINT(pipeline_name)
    if is_nolint_suppressed(source, node, pipeline_name) {
        return true;
    }
    // Check #pragma warning disable (blanket)
    if is_pragma_suppressed(source, node, &[]) {
        return true;
    }
    // Check [SuppressMessage] attribute on the node or its parent declaration
    if has_csharp_attribute(node, source, "SuppressMessage") {
        return true;
    }
    // Also check the parent (e.g., finding is on a statement inside a method with [SuppressMessage])
    if let Some(parent) = node.parent() {
        if has_csharp_attribute(parent, source, "SuppressMessage") {
            return true;
        }
    }
    false
}

/// Check if a file contains generated code (designer files, auto-generated).
pub fn is_generated_code(file_path: &str, source: &[u8]) -> bool {
    // File extension patterns
    if file_path.ends_with(".g.cs")
        || file_path.ends_with(".designer.cs")
        || file_path.ends_with(".Designer.cs")
        || file_path.contains("/Migrations/")
    {
        return true;
    }
    // Check for <auto-generated> header comment in first 5 lines
    let source_str = match std::str::from_utf8(source) {
        Ok(s) => s,
        Err(_) => return false,
    };
    for line in source_str.lines().take(5) {
        if line.contains("<auto-generated") || line.contains("auto-generated>") {
            return true;
        }
    }
    false
}

/// Check if a method has an event handler signature: (object sender, EventArgs e) or similar.
pub fn is_event_handler_signature(method_node: tree_sitter::Node, source: &[u8]) -> bool {
    let params = match method_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return false,
    };
    let param_text = params.utf8_text(source).unwrap_or("");
    // Standard event handler patterns
    param_text.contains("object sender") && param_text.contains("EventArgs")
}

/// Shared DTO/data class detection by class name suffix and file path.
const DTO_SUFFIXES: &[&str] = &[
    "Dto",
    "DTO",
    "ViewModel",
    "Request",
    "Response",
    "Command",
    "Query",
    "Event",
    "Message",
    "Options",
    "Settings",
    "Config",
    "Configuration",
    "Entity",
    "Model",
    "Record",
    "State",
    "Args",
    "Params",
    "Result",
    "Payload",
    "Schema",
    "Spec",
];

const DTO_PATH_PATTERNS: &[&str] = &[
    "/Models/",
    "/Entities/",
    "/DTOs/",
    "/Dtos/",
    "/ViewModels/",
    "/Requests/",
    "/Responses/",
    "/Events/",
    "/Messages/",
];

pub fn is_dto_or_data_class(class_name: &str, file_path: &str) -> bool {
    // Check name suffixes
    for suffix in DTO_SUFFIXES {
        if class_name.ends_with(suffix) && class_name.len() > suffix.len() {
            return true;
        }
    }
    // Check file path patterns
    let path = file_path.replace('\\', "/");
    for pattern in DTO_PATH_PATTERNS {
        if path.contains(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_csharp(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.as_bytes().to_vec())
    }

    fn count_matches(query: &Query, tree: &tree_sitter::Tree, source: &[u8]) -> usize {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        let mut count = 0;
        while matches.next().is_some() {
            count += 1;
        }
        count
    }

    #[test]
    fn class_decl_compiles_and_matches() {
        let src = "class Foo { }";
        let (tree, source) = parse_csharp(src);
        let query = compile_class_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_decl_compiles_and_matches() {
        let src = "class Foo { public void Bar() { } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_method_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn field_decl_compiles_and_matches() {
        let src = "class Foo { private int _x; }";
        let (tree, source) = parse_csharp(src);
        let query = compile_field_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn property_decl_compiles_and_matches() {
        let src = "class Foo { public string Name { get; set; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_property_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn catch_clause_compiles_and_matches() {
        let src = "class Foo { void M() { try { } catch (Exception e) { } } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_catch_clause_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn return_null_compiles_and_matches() {
        let src = "class Foo { object M() { return null; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_return_null_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn invocation_compiles_and_matches() {
        let src = "class Foo { void M() { Console.WriteLine(\"hi\"); } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_invocation_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn member_access_compiles_and_matches() {
        let src = "class Foo { void M() { var x = obj.Name; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_member_access_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn parameter_compiles_and_matches() {
        let src = "class Foo { void M(string name, int age) { } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_parameter_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn string_literal_compiles_and_matches() {
        let src = "class Foo { string s = \"hello\"; }";
        let (tree, source) = parse_csharp(src);
        let query = compile_string_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn object_creation_compiles_and_matches() {
        let src = "class Foo { void M() { new SqlCommand(\"SELECT 1\", conn); } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_object_creation_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn binary_expression_compiles_and_matches() {
        let src = "class Foo { void M() { var x = 1 + 2; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_binary_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn assignment_expression_compiles_and_matches() {
        let src = "class Foo { void M() { int x; x = 1; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_assignment_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn interpolated_string_compiles_and_matches() {
        let src = "class Foo { void M() { var s = $\"hello {name}\"; } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_interpolated_string_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_with_body_compiles_and_matches() {
        let src = "class Foo { void Bar() { int x = 1; } Foo() { } }";
        let (tree, source) = parse_csharp(src);
        let query = compile_method_with_body_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn has_modifier_detects_public() {
        let src = "class Foo { public int x; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_modifier(field, &source, "public"));
        assert!(!has_modifier(field, &source, "private"));
    }

    #[test]
    fn has_modifier_detects_static_readonly() {
        let src = "class Foo { private static readonly int x = 1; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_modifier(field, &source, "private"));
        assert!(has_modifier(field, &source, "static"));
        assert!(has_modifier(field, &source, "readonly"));
        assert!(!has_modifier(field, &source, "const"));
    }

    // ── Suppression helper tests ──────────────────────────

    #[test]
    fn pragma_blanket_suppresses() {
        let src = "#pragma warning disable\nclass Foo { public int x; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(is_pragma_suppressed(&source, class_node, &[]));
    }

    #[test]
    fn pragma_targeted_matches_code() {
        let src = "#pragma warning disable CS0618, CA1031\nclass Foo { }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(is_pragma_suppressed(&source, class_node, &["CA1031"]));
        assert!(!is_pragma_suppressed(&source, class_node, &["CS9999"]));
    }

    #[test]
    fn pragma_no_match_without_pragma() {
        let src = "class Foo { }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(!is_pragma_suppressed(&source, class_node, &[]));
    }

    #[test]
    fn has_csharp_attribute_detects_threadstatic() {
        let src = "class Foo { [ThreadStatic] private static int _x; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_csharp_attribute(field, &source, "ThreadStatic"));
        assert!(!has_csharp_attribute(field, &source, "Obsolete"));
    }

    #[test]
    fn is_generated_code_detects_designer_files() {
        assert!(is_generated_code("Foo.designer.cs", b""));
        assert!(is_generated_code("Foo.Designer.cs", b""));
        assert!(is_generated_code("Foo.g.cs", b""));
        assert!(is_generated_code("src/Migrations/001_Init.cs", b""));
        assert!(!is_generated_code("src/Foo.cs", b""));
    }

    #[test]
    fn is_generated_code_detects_auto_generated_header() {
        let src = b"// <auto-generated />\nclass Foo { }";
        assert!(is_generated_code("Foo.cs", src));
    }

    #[test]
    fn is_event_handler_detects_standard_pattern() {
        let src = "class Foo { async void OnClick(object sender, EventArgs e) { } }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let method = class_body.named_child(0).unwrap();
        assert!(is_event_handler_signature(method, &source));
    }

    #[test]
    fn is_event_handler_rejects_normal_method() {
        let src = "class Foo { async Task DoWork(string name) { } }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let method = class_body.named_child(0).unwrap();
        assert!(!is_event_handler_signature(method, &source));
    }

    #[test]
    fn dto_detection_by_suffix() {
        assert!(is_dto_or_data_class("OrderDto", "src/Order.cs"));
        assert!(is_dto_or_data_class("UserViewModel", "src/User.cs"));
        assert!(is_dto_or_data_class("CreateOrderRequest", "src/Order.cs"));
        assert!(is_dto_or_data_class("OrderEntity", "src/Order.cs"));
        assert!(!is_dto_or_data_class("OrderService", "src/Order.cs"));
        // Suffix alone should not match (e.g., "Dto" by itself)
        assert!(!is_dto_or_data_class("Dto", "src/Dto.cs"));
    }

    #[test]
    fn dto_detection_by_path() {
        assert!(is_dto_or_data_class("Order", "src/Models/Order.cs"));
        assert!(is_dto_or_data_class("Order", "src/Entities/Order.cs"));
        assert!(is_dto_or_data_class("Order", "src/DTOs/Order.cs"));
        assert!(!is_dto_or_data_class("Order", "src/Services/Order.cs"));
    }

    #[test]
    fn combined_suppression_nolint() {
        let src = "// NOLINT\nclass Foo { public int x; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(is_csharp_suppressed(&source, class_node, "test_pipeline"));
    }

    #[test]
    fn combined_suppression_pragma() {
        let src = "#pragma warning disable\nclass Foo { }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(is_csharp_suppressed(&source, class_node, "test_pipeline"));
    }

    #[test]
    fn no_suppression_when_clean() {
        let src = "class Foo { }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_node = root.named_child(0).unwrap();
        assert!(!is_csharp_suppressed(&source, class_node, "test_pipeline"));
    }
}
