use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

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
        if child.kind() == "modifier" {
            if child.utf8_text(source).unwrap_or("") == modifier_text {
                return true;
            }
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
    fn has_modifier_detects_public() {
        let src = "class Foo { public int x; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root.named_child(0).unwrap().child_by_field_name("body").unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_modifier(field, &source, "public"));
        assert!(!has_modifier(field, &source, "private"));
    }

    #[test]
    fn has_modifier_detects_static_readonly() {
        let src = "class Foo { private static readonly int x = 1; }";
        let (tree, source) = parse_csharp(src);
        let root = tree.root_node();
        let class_body = root.named_child(0).unwrap().child_by_field_name("body").unwrap();
        let field = class_body.named_child(0).unwrap();
        assert!(has_modifier(field, &source, "private"));
        assert!(has_modifier(field, &source, "static"));
        assert!(has_modifier(field, &source, "readonly"));
        assert!(!has_modifier(field, &source, "const"));
    }
}
