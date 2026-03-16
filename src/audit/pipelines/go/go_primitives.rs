use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

pub fn compile_short_var_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(short_var_declaration
  left: (expression_list) @lhs
  right: (expression_list) @rhs) @decl
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile short_var_declaration query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_assignment_query() -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_statement
  left: (expression_list) @lhs
  right: (expression_list) @rhs) @assign
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile assignment query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_struct_type_query() -> Result<Arc<Query>> {
    let query_str = r#"
(type_declaration
  (type_spec
    name: (type_identifier) @struct_name
    type: (struct_type
      (field_declaration_list) @fields))) @type_decl
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile struct type query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_method_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: (_) @receiver_type))
  name: (field_identifier) @method_name) @method_decl
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile method declaration query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_function_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_declaration
  name: (identifier) @fn_name
  body: (block) @fn_body) @fn_decl
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile function declaration query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_selector_call_query() -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @method)) @call
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile selector call query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_method_call_query() -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (selector_expression
    field: (field_identifier) @method_name)) @call
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile method call query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_go_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(go_statement
  (_) @go_expr) @go_stmt
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile go statement query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_param_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(parameter_declaration
  name: (identifier)? @param_name
  type: (_) @param_type) @param
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile parameter declaration query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_field_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(field_declaration
  name: (field_identifier)? @field_name
  type: (_) @field_type) @field
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile field declaration query for Go")?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
[(int_literal) @number (float_literal) @number]
"#;
    let query = Query::new(&go_lang(), query_str)
        .with_context(|| "failed to compile numeric literal query for Go")?;
    Ok(Arc::new(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_go(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
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
    fn short_var_decl_compiles_and_matches() {
        let src = "package main\nfunc f() { x := 1 }";
        let (tree, source) = parse_go(src);
        let query = compile_short_var_decl_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn assignment_compiles_and_matches() {
        let src = "package main\nfunc f() { var x int; x = 1 }";
        let (tree, source) = parse_go(src);
        let query = compile_assignment_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn struct_type_compiles_and_matches() {
        let src = "package main\ntype Foo struct { X int; Y string }";
        let (tree, source) = parse_go(src);
        let query = compile_struct_type_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_decl_compiles_and_matches() {
        let src = "package main\ntype S struct{}\nfunc (s S) M() {}";
        let (tree, source) = parse_go(src);
        let query = compile_method_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_decl_compiles_and_matches() {
        let src = "package main\nfunc hello() { return }";
        let (tree, source) = parse_go(src);
        let query = compile_function_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn selector_call_compiles_and_matches() {
        let src = "package main\nimport \"fmt\"\nfunc f() { fmt.Println(\"hi\") }";
        let (tree, source) = parse_go(src);
        let query = compile_selector_call_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn method_call_compiles_and_matches() {
        let src = "package main\nfunc f() { s.Close() }";
        let (tree, source) = parse_go(src);
        let query = compile_method_call_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn go_statement_compiles_and_matches() {
        let src = "package main\nfunc f() { go func() {}() }";
        let (tree, source) = parse_go(src);
        let query = compile_go_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn param_decl_compiles_and_matches() {
        let src = "package main\nfunc f(x int, y string) {}";
        let (tree, source) = parse_go(src);
        let query = compile_param_decl_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 2);
    }

    #[test]
    fn field_decl_compiles_and_matches() {
        let src = "package main\ntype S struct { X int; Y string }";
        let (tree, source) = parse_go(src);
        let query = compile_field_decl_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 2);
    }

    #[test]
    fn numeric_literal_compiles_and_matches() {
        let src = "package main\nfunc f() { x := 42; y := 3.14 }";
        let (tree, source) = parse_go(src);
        let query = compile_numeric_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }
}
