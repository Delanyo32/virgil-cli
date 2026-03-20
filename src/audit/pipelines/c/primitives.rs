use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub fn compile_call_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (argument_list) @args) @call
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile call_expression query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_function_definition_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile function_definition query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
(number_literal) @number
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile numeric_literal query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(declaration) @decl
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile declaration query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_type_definition_query() -> Result<Arc<Query>> {
    let query_str = r#"
(type_definition
  type: (_) @typedef_type
  declarator: (_) @typedef_name) @typedef_decl
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile type_definition query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_preproc_function_def_query() -> Result<Arc<Query>> {
    let query_str = r#"
(preproc_function_def
  name: (identifier) @macro_name
  parameters: (preproc_params) @macro_params) @macro_def
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile preproc_function_def query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_expression_statement_call_query() -> Result<Arc<Query>> {
    let query_str = r#"
(expression_statement
  (call_expression
    function: (identifier) @fn_name
    arguments: (argument_list) @args) @call) @expr_stmt
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile expression_statement_call query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_parameter_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(parameter_declaration
  type: (_) @param_type
  declarator: (_)? @param_declarator) @param_decl
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile parameter_declaration query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_for_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(for_statement
  initializer: (declaration) @for_init
  condition: (_)? @for_cond) @for_stmt
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile for_statement query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_if_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(if_statement
  condition: (parenthesized_expression) @condition
  consequence: (compound_statement) @if_body) @if_stmt
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile if_statement query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression
  left: (_) @left
  right: (_) @right) @bin_expr
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile binary_expression query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_string_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
(string_literal) @string_lit
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile string_literal query for C")?;
    Ok(Arc::new(query))
}

pub fn compile_return_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(return_statement) @return_stmt
"#;
    let query = Query::new(&c_lang(), query_str)
        .with_context(|| "failed to compile return_statement query for C")?;
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

pub fn find_identifier_in_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_identifier_in_declarator(inner, source);
    }
    None
}

pub fn is_pointer_declarator(node: tree_sitter::Node) -> bool {
    if node.kind() == "pointer_declarator" {
        return true;
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return is_pointer_declarator(inner);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_c(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
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
    fn call_expression_compiles_and_matches() {
        let src = "void f() { printf(\"hello\"); }";
        let (tree, source) = parse_c(src);
        let query = compile_call_expression_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn function_definition_compiles_and_matches() {
        let src = "int main() { return 0; }";
        let (tree, source) = parse_c(src);
        let query = compile_function_definition_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn numeric_literal_compiles_and_matches() {
        let src = "void f() { int x = 42; float y = 3.14; }";
        let (tree, source) = parse_c(src);
        let query = compile_numeric_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn declaration_compiles_and_matches() {
        let src = "int x = 0; const int y = 1;";
        let (tree, source) = parse_c(src);
        let query = compile_declaration_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn type_definition_compiles_and_matches() {
        let src = "typedef unsigned int uint;";
        let (tree, source) = parse_c(src);
        let query = compile_type_definition_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn preproc_function_def_compiles_and_matches() {
        let src = "#define ADD(a, b) ((a) + (b))";
        let (tree, source) = parse_c(src);
        let query = compile_preproc_function_def_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn expression_statement_call_compiles_and_matches() {
        let src = "void f() { printf(\"hello\"); }";
        let (tree, source) = parse_c(src);
        let query = compile_expression_statement_call_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn parameter_declaration_compiles_and_matches() {
        let src = "void f(int x, char *y) {}";
        let (tree, source) = parse_c(src);
        let query = compile_parameter_declaration_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 2);
    }

    #[test]
    fn for_statement_compiles_and_matches() {
        let src = "void f() { for (int i = 0; i < 10; i++) {} }";
        let (tree, source) = parse_c(src);
        let query = compile_for_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn has_type_qualifier_const() {
        let src = "const int x = 0;";
        let (tree, _source) = parse_c(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert!(has_type_qualifier(decl, src.as_bytes(), "const"));
    }

    #[test]
    fn has_storage_class_static() {
        let src = "static int x = 0;";
        let (tree, _source) = parse_c(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        assert!(has_storage_class(decl, src.as_bytes(), "static"));
    }

    #[test]
    fn find_identifier_in_pointer_declarator() {
        let src = "int *ptr = 0;";
        let (tree, _source) = parse_c(src);
        let root = tree.root_node();
        let decl = root.named_child(0).unwrap();
        // The declarator is an init_declarator wrapping a pointer_declarator
        let declarator = decl.child_by_field_name("declarator").unwrap();
        let name = find_identifier_in_declarator(declarator, src.as_bytes());
        assert_eq!(name.as_deref(), Some("ptr"));
    }
}
