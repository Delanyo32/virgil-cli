use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

pub fn find_capture_index(query: &Query, name: &str) -> usize {
    query
        .capture_names()
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("query must have @{name} capture"))
}

pub fn node_text<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

pub fn extract_snippet(source: &[u8], node: tree_sitter::Node, max_lines: usize) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        text.to_string()
    } else {
        let mut snippet: String = lines[..max_lines].join("\n");
        snippet.push_str("\n...");
        snippet
    }
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
