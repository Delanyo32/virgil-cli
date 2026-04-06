use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub fn compile_new_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(new_expression) @new_expr
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile new_expression query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_delete_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(delete_expression) @delete_expr
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile delete_expression query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_class_specifier_query() -> Result<Arc<Query>> {
    let query_str = r#"
(class_specifier
  name: (type_identifier) @class_name
  body: (field_declaration_list) @class_body) @class_def
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile class_specifier query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_struct_specifier_query() -> Result<Arc<Query>> {
    let query_str = r#"
(struct_specifier
  name: (type_identifier) @struct_name
  body: (field_declaration_list) @struct_body) @struct_def
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile struct_specifier query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_cast_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(cast_expression
  type: (_) @cast_type
  value: (_) @cast_value) @cast_expr
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile cast_expression query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_parameter_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(parameter_declaration
  type: (_) @param_type
  declarator: (_)? @param_declarator) @param_decl
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile parameter_declaration query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_qualified_identifier_query() -> Result<Arc<Query>> {
    let query_str = r#"
(qualified_identifier) @qualified_id
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile qualified_identifier query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_union_specifier_query() -> Result<Arc<Query>> {
    let query_str = r#"
(union_specifier
  name: (type_identifier)? @union_name) @union_def
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile union_specifier query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_preproc_include_query() -> Result<Arc<Query>> {
    let query_str = r#"
(preproc_include) @include_dir
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile preproc_include query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_throw_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(throw_statement) @throw_stmt
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile throw_statement query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_field_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_declaration
  type: (_) @field_type
  declarator: (_) @field_declarator) @field_decl
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile field_declaration query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
(number_literal) @number
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile numeric_literal query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_function_definition_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile function_definition query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(declaration) @decl
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile declaration query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_if_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(if_statement
  condition: (condition_clause) @condition
  consequence: (compound_statement) @if_body) @if_stmt
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile if_statement query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression
  left: (_) @left
  right: (_) @right) @bin_expr
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile binary_expression query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_call_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (_) @fn_name
  arguments: (argument_list) @args) @call
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile call_expression query for C++")?;
    Ok(Arc::new(query))
}

pub fn compile_return_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(return_statement) @return_stmt
"#;
    let query = Query::new(&cpp_lang(), query_str)
        .with_context(|| "failed to compile return_statement query for C++")?;
    Ok(Arc::new(query))
}

// ── Helper functions ──

pub fn has_type_qualifier(node: tree_sitter::Node, source: &[u8], qualifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_qualifier" && node_text(child, source) == qualifier {
            return true;
        }
    }
    false
}

pub fn has_storage_class(node: tree_sitter::Node, source: &[u8], class: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier" && node_text(child, source) == class {
            return true;
        }
    }
    false
}

pub fn has_constexpr(node: tree_sitter::Node, source: &[u8]) -> bool {
    let text = node_text(node, source);
    // In tree-sitter-cpp, constexpr may appear as storage_class_specifier or type_qualifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_text = node_text(child, source);
        if child_text == "constexpr" {
            return true;
        }
    }
    // Also check the full declaration text as fallback
    text.contains("constexpr")
}

pub fn is_reference_declarator(node: tree_sitter::Node) -> bool {
    if node.kind() == "reference_declarator" {
        return true;
    }
    if node.kind() == "abstract_reference_declarator" {
        return true;
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return is_reference_declarator(inner);
    }
    false
}

pub fn is_pointer_declarator(node: tree_sitter::Node) -> bool {
    if node.kind() == "pointer_declarator" {
        return true;
    }
    if node.kind() == "abstract_pointer_declarator" {
        return true;
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return is_pointer_declarator(inner);
    }
    false
}

pub fn is_inside_node_kind(node: tree_sitter::Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return true;
        }
        current = parent.parent();
    }
    false
}

pub fn find_identifier_in_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    if node.kind() == "destructor_name" || node.kind() == "operator_name" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_identifier_in_declarator(inner, source);
    }
    // Walk children for cases like qualified_identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "field_identifier" {
            return child.utf8_text(source).ok().map(|s| s.to_string());
        }
    }
    None
}

// ── Shared analysis helpers ──

/// Check if a node is inside a loop (for, while, do, range-for).
pub fn is_inside_loop(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "for_statement" | "while_statement" | "do_statement" | "for_range_loop" => {
                return true;
            }
            _ => {}
        }
        current = parent.parent();
    }
    false
}

/// Check if a node is inside a try block.
pub fn is_inside_try_block(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "try_statement" {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Check if the translation unit has a `using namespace std;` directive.
pub fn has_using_namespace_std(root: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "using_declaration" {
            let text = node_text(child, source);
            if text.contains("namespace") && text.contains("std") {
                return true;
            }
        }
    }
    false
}

/// Returns true if the file is generated (protobuf, Qt MOC/QRC/UI, CMake autogen,
/// vendored/build directories, or a source comment in the first 10 lines signals it).
pub fn is_generated_cpp_file(file_path: &str, source: &[u8]) -> bool {
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");

    // Protobuf
    if file_name.ends_with(".pb.h") || file_name.ends_with(".pb.cc") {
        return true;
    }
    // Qt MOC and QRC
    if file_name.starts_with("moc_") || file_name.starts_with("qrc_") {
        return true;
    }
    // Qt UI-to-header
    if file_name.starts_with("ui_") && (file_name.ends_with(".h") || file_name.ends_with(".hpp")) {
        return true;
    }
    // CMake autogen suffix
    if file_name.ends_with("_autogen.cpp") {
        return true;
    }

    // Directory-based exclusions
    let path = file_path.replace('\\', "/");
    if path.contains("/vendor/")
        || path.starts_with("vendor/")
        || path.contains("/third_party/")
        || path.starts_with("third_party/")
        || path.contains("/build/")
        || path.starts_with("build/")
        || path.contains("/_deps/")
        || path.starts_with("_deps/")
        || path.contains("/generated/")
        || path.starts_with("generated/")
        || path.contains("_autogen/")
    {
        return true;
    }

    // Source comment scan — first 10 lines
    if let Ok(s) = std::str::from_utf8(source) {
        let header = s.lines().take(10).collect::<Vec<_>>().join("\n").to_lowercase();
        if header.contains("generated")
            || header.contains("auto-generated")
            || header.contains("do not edit")
        {
            return true;
        }
    }

    false
}

/// Returns true if `file_path` has a C++-specific header extension: `.hpp`, `.hxx`, or `.hh`.
/// `.h` files are deliberately excluded — they are ambiguous (C or C++) and handled by C pipelines.
pub fn is_cpp_header(file_path: &str) -> bool {
    file_path.ends_with(".hpp") || file_path.ends_with(".hxx") || file_path.ends_with(".hh")
}

/// Returns true if `decl` (a tree-sitter `declaration` node) is a forward declaration
/// (no body, no initializer). Mirrors `is_c_forward_declaration()` in `c/primitives.rs`.
///
/// Forward declarations: `class Foo;`, `struct Bar;`, `void func();`
/// Not forward declarations: `class Foo { ... };`, `int x = 42;`
pub fn is_cpp_forward_declaration(decl: tree_sitter::Node) -> bool {
    let mut cursor = decl.walk();
    for child in decl.children(&mut cursor) {
        let kind = child.kind();
        if kind == "compound_statement" || kind == "init_declarator" {
            return false;
        }
        if (kind == "class_specifier" || kind == "struct_specifier")
            && child.child_by_field_name("body").is_some()
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_cpp(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
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
    fn new_expression_compiles_and_matches() {
        let src = "void f() { int* p = new int; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_new_expression_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn delete_expression_compiles_and_matches() {
        let src = "void f() { delete p; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_delete_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn class_specifier_compiles_and_matches() {
        let src = "class Foo { int x; };";
        let (tree, source) = parse_cpp(src);
        let query = compile_class_specifier_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn struct_specifier_compiles_and_matches() {
        let src = "struct Bar { int x; };";
        let (tree, source) = parse_cpp(src);
        let query = compile_struct_specifier_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn cast_expression_compiles_and_matches() {
        let src = "void f() { int x = (int)3.14; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_cast_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn parameter_declaration_compiles_and_matches() {
        let src = "void f(int x, double y) {}";
        let (tree, source) = parse_cpp(src);
        let query = compile_parameter_declaration_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 2);
    }

    #[test]
    fn qualified_identifier_compiles_and_matches() {
        let src = "void f() { std::cout << std::endl; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_qualified_identifier_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn union_specifier_compiles_and_matches() {
        let src = "union Data { int i; float f; };";
        let (tree, source) = parse_cpp(src);
        let query = compile_union_specifier_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn preproc_include_compiles_and_matches() {
        let src = "#include <iostream>\n#include \"myheader.h\"";
        let (tree, source) = parse_cpp(src);
        let query = compile_preproc_include_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn throw_statement_compiles_and_matches() {
        let src = "void f() { throw 42; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_throw_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn field_declaration_compiles_and_matches() {
        let src = "class Foo { int x; double y; };";
        let (tree, source) = parse_cpp(src);
        let query = compile_field_declaration_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn numeric_literal_compiles_and_matches() {
        let src = "void f() { int x = 42; float y = 3.14; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_numeric_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn function_definition_compiles_and_matches() {
        let src = "int main() { return 0; }";
        let (tree, source) = parse_cpp(src);
        let query = compile_function_definition_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn declaration_compiles_and_matches() {
        let src = "int x = 0; const int y = 1;";
        let (tree, source) = parse_cpp(src);
        let query = compile_declaration_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn has_type_qualifier_const() {
        let src = "const int x = 0;";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert!(has_type_qualifier(decl, src.as_bytes(), "const"));
    }

    #[test]
    fn has_storage_class_static() {
        let src = "static int x = 0;";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert!(has_storage_class(decl, src.as_bytes(), "static"));
    }

    #[test]
    fn is_inside_node_kind_works() {
        let src = "class Foo { void bar() { int x = 0; } };";
        let (tree, _source) = parse_cpp(src);
        // Find the declaration inside the method
        let root = tree.root_node();
        // Navigate: root -> class declaration -> class_specifier -> field_declaration_list -> function_definition -> compound_statement -> declaration
        fn find_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == kind {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(found) = find_kind(child, kind) {
                    return Some(found);
                }
            }
            None
        }
        let decl = find_kind(root, "declaration").unwrap();
        assert!(is_inside_node_kind(decl, "class_specifier"));
    }

    // ── Tests for new C++ helpers ──────────────────────────────────────

    #[test]
    fn is_generated_cpp_file_protobuf_header() {
        assert!(is_generated_cpp_file("proto/user.pb.h", b"struct Foo {};"));
    }

    #[test]
    fn is_generated_cpp_file_protobuf_source() {
        assert!(is_generated_cpp_file("src/user.pb.cc", b"void Foo::Init() {}"));
    }

    #[test]
    fn is_generated_cpp_file_moc() {
        assert!(is_generated_cpp_file("moc_mainwindow.cpp", b"void setup() {}"));
    }

    #[test]
    fn is_generated_cpp_file_qrc() {
        assert!(is_generated_cpp_file("qrc_resources.cpp", b"void init() {}"));
    }

    #[test]
    fn is_generated_cpp_file_ui_header() {
        assert!(is_generated_cpp_file("ui_mainwindow.h", b"void setup() {}"));
    }

    #[test]
    fn is_generated_cpp_file_autogen_suffix() {
        assert!(is_generated_cpp_file("foo_autogen.cpp", b"void f() {}"));
    }

    #[test]
    fn is_generated_cpp_file_autogen_dir() {
        assert!(is_generated_cpp_file("src/CMakeFiles/foo_autogen/init.cpp", b"void f() {}"));
    }

    #[test]
    fn is_generated_cpp_file_vendor_dir() {
        assert!(is_generated_cpp_file("src/vendor/lib/module.cpp", b"void f() {}"));
    }

    #[test]
    fn is_generated_cpp_file_third_party_dir() {
        assert!(is_generated_cpp_file("third_party/protobuf/message.cpp", b"void f() {}"));
    }

    #[test]
    fn is_generated_cpp_file_build_dir() {
        assert!(is_generated_cpp_file("build/generated/foo.cpp", b"void f() {}"));
    }

    #[test]
    fn is_generated_cpp_file_deps_dir() {
        assert!(is_generated_cpp_file("src/_deps/catch2/catch.hpp", b"// catch"));
    }

    #[test]
    fn is_generated_cpp_file_by_source_comment() {
        let src = b"// Auto-generated by protoc. Do not edit.\n#include \"user.pb.h\"\n";
        assert!(is_generated_cpp_file("user_impl.cpp", src));
    }

    #[test]
    fn is_generated_cpp_file_normal_file_is_not() {
        let src = b"#include <iostream>\nvoid greet() { std::cout << \"hello\"; }\n";
        assert!(!is_generated_cpp_file("greeter.cpp", src));
    }

    #[test]
    fn is_cpp_header_hpp() {
        assert!(is_cpp_header("include/foo.hpp"));
    }

    #[test]
    fn is_cpp_header_hxx() {
        assert!(is_cpp_header("src/bar.hxx"));
    }

    #[test]
    fn is_cpp_header_hh() {
        assert!(is_cpp_header("types.hh"));
    }

    #[test]
    fn is_cpp_header_not_for_h() {
        assert!(!is_cpp_header("legacy.h"));
    }

    #[test]
    fn is_cpp_header_not_for_cpp() {
        assert!(!is_cpp_header("main.cpp"));
    }

    #[test]
    fn is_cpp_forward_declaration_class_no_body() {
        // In tree-sitter C++, `class Foo;` parses as class_specifier (not declaration).
        // A class forward declaration has no body field on the class_specifier.
        let src = "class Foo;";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let node = root.named_child(0).unwrap();
        assert_eq!(node.kind(), "class_specifier");
        assert!(node.child_by_field_name("body").is_none());
    }

    #[test]
    fn is_cpp_forward_declaration_class_with_body_is_not() {
        // `class Foo { int x; };` parses as class_specifier with a body field.
        let src = "class Foo { int x; };";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let node = root.named_child(0).unwrap();
        assert_eq!(node.kind(), "class_specifier");
        assert!(node.child_by_field_name("body").is_some());
    }

    #[test]
    fn is_cpp_forward_declaration_function_prototype() {
        // Function prototypes like `void foo(int x);` are declaration nodes.
        let src = "void foo(int x);";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert_eq!(decl.kind(), "declaration");
        assert!(is_cpp_forward_declaration(decl));
    }

    #[test]
    fn is_cpp_forward_declaration_variable_with_init_is_not() {
        let src = "int x = 42;";
        let (tree, _source) = parse_cpp(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert_eq!(decl.kind(), "declaration");
        assert!(!is_cpp_forward_declaration(decl));
    }
}
