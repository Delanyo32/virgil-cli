use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

/// Check if a node (e.g. field_declaration, method_declaration) has a specific modifier
/// by inspecting its `modifiers` child node.
pub fn has_modifier(node: tree_sitter::Node, source: &[u8], modifier_text: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for modifier in child.children(&mut mod_cursor) {
                if modifier.utf8_text(source).unwrap_or("") == modifier_text {
                    return true;
                }
            }
        }
    }
    false
}

pub fn compile_class_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(class_declaration
  name: (identifier) @class_name
  body: (class_body) @class_body) @class_decl
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile class_declaration query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_field_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @field_name)) @field_decl
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile field_declaration query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_catch_clause_query() -> Result<Arc<Query>> {
    let query_str = r#"
(catch_clause
  body: (block) @catch_body) @catch
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile catch_clause query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_return_null_query() -> Result<Arc<Query>> {
    let query_str = r#"
(return_statement (null_literal) @null_val) @return_stmt
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile return_null query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_local_var_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(local_variable_declaration
  type: (_) @var_type
  declarator: (variable_declarator
    name: (identifier) @var_name
    value: (object_creation_expression)? @creation)) @var_decl
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile local_variable_declaration query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_raw_type_field_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_declaration
  type: (type_identifier) @raw_type
  declarator: (variable_declarator
    name: (identifier) @var_name)) @decl
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile raw_type_field query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_raw_type_local_query() -> Result<Arc<Query>> {
    let query_str = r#"
(local_variable_declaration
  type: (type_identifier) @raw_type
  declarator: (variable_declarator
    name: (identifier) @var_name)) @decl
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile raw_type_local query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_raw_type_param_query() -> Result<Arc<Query>> {
    let query_str = r#"
(formal_parameter
  type: (type_identifier) @raw_type
  name: (identifier) @param_name) @param
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile raw_type_param query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_if_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(if_statement
  condition: (parenthesized_expression) @condition) @if_stmt
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile if_statement query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_method_invocation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(method_invocation
  name: (identifier) @method_name
  arguments: (argument_list) @args) @invocation
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile method_invocation query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_assignment_query() -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_expression
  left: (identifier) @lhs
  right: (_) @rhs) @assign
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile assignment_expression query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_object_creation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(object_creation_expression
  type: (_) @type_name
  arguments: (argument_list) @args) @creation
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile object_creation_expression query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression
  left: (_) @left
  right: (_) @right) @bin_expr
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile binary_expression query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_method_invocation_with_object_query() -> Result<Arc<Query>> {
    let query_str = r#"
(method_invocation
  object: (_) @object
  name: (identifier) @method_name
  arguments: (argument_list) @args) @invocation
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile method_invocation_with_object query for Java")?;
    Ok(Arc::new(query))
}

pub fn compile_field_access_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_access
  object: (_) @object
  field: (identifier) @field_name) @access
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile field_access query for Java")?;
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
    body: (constructor_body) @method_body) @method
]
"#;
    let query = Query::new(&java_lang(), query_str)
        .with_context(|| "failed to compile method_with_body query for Java")?;
    Ok(Arc::new(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_java(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&java_lang()).unwrap();
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
        let (tree, source) = parse_java(src);
        let query = compile_class_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn field_decl_compiles_and_matches() {
        let src = "class Foo { private int x; }";
        let (tree, source) = parse_java(src);
        let query = compile_field_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn catch_clause_compiles_and_matches() {
        let src = "class Foo { void m() { try { } catch (Exception e) { } } }";
        let (tree, source) = parse_java(src);
        let query = compile_catch_clause_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn return_null_compiles_and_matches() {
        let src = "class Foo { Object m() { return null; } }";
        let (tree, source) = parse_java(src);
        let query = compile_return_null_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn local_var_decl_compiles_and_matches() {
        let src = "class Foo { void m() { String s = new String(); } }";
        let (tree, source) = parse_java(src);
        let query = compile_local_var_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn raw_type_field_compiles_and_matches() {
        let src = "class Foo { List items; }";
        let (tree, source) = parse_java(src);
        let query = compile_raw_type_field_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn raw_type_local_compiles_and_matches() {
        let src = "class Foo { void m() { List items = null; } }";
        let (tree, source) = parse_java(src);
        let query = compile_raw_type_local_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn raw_type_param_compiles_and_matches() {
        let src = "class Foo { void m(List items) { } }";
        let (tree, source) = parse_java(src);
        let query = compile_raw_type_param_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn if_statement_compiles_and_matches() {
        let src = "class Foo { void m() { if (true) { } } }";
        let (tree, source) = parse_java(src);
        let query = compile_if_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_invocation_compiles_and_matches() {
        let src = "class Foo { void m() { s.equals(\"x\"); } }";
        let (tree, source) = parse_java(src);
        let query = compile_method_invocation_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn assignment_compiles_and_matches() {
        let src = "class Foo { void m() { int x; x = 1; } }";
        let (tree, source) = parse_java(src);
        let query = compile_assignment_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn has_modifier_detects_public() {
        let src = "class Foo { public int x; }";
        let (tree, source) = parse_java(src);
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
    fn has_modifier_detects_private() {
        let src = "class Foo { private final int x = 1; }";
        let (tree, source) = parse_java(src);
        let root = tree.root_node();
        let class_body = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_modifier(field, &source, "private"));
        assert!(has_modifier(field, &source, "final"));
        assert!(!has_modifier(field, &source, "public"));
    }

    #[test]
    fn object_creation_compiles_and_matches() {
        let src = "class Foo { void m() { new String(\"hello\"); } }";
        let (tree, source) = parse_java(src);
        let query = compile_object_creation_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn binary_expression_compiles_and_matches() {
        let src = "class Foo { void m() { int x = 1 + 2; } }";
        let (tree, source) = parse_java(src);
        let query = compile_binary_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_invocation_with_object_compiles_and_matches() {
        let src = "class Foo { void m() { stmt.executeQuery(sql); } }";
        let (tree, source) = parse_java(src);
        let query = compile_method_invocation_with_object_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn field_access_compiles_and_matches() {
        let src = "class Foo { void m() { int x = Math.PI; } }";
        let (tree, source) = parse_java(src);
        let query = compile_field_access_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn method_with_body_compiles_and_matches() {
        let src = "class Foo { void bar() { int x = 1; } Foo() { } }";
        let (tree, source) = parse_java(src);
        let query = compile_method_with_body_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }
}
