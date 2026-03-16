use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
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

pub fn compile_function_def_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_definition
  name: (identifier) @fn_name
  parameters: (parameters) @params
  body: (block) @fn_body) @fn_def
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile function_definition query for Python")?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
[(integer) @number (float) @number]
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile numeric literal query for Python")?;
    Ok(Arc::new(query))
}

pub fn compile_except_clause_query() -> Result<Arc<Query>> {
    let query_str = r#"
(except_clause) @except
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile except_clause query for Python")?;
    Ok(Arc::new(query))
}

pub fn compile_default_parameter_query() -> Result<Arc<Query>> {
    let query_str = r#"
[
  (default_parameter) @default_param
  (typed_default_parameter) @default_param
]
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile default parameter query for Python")?;
    Ok(Arc::new(query))
}

pub fn compile_comparison_query() -> Result<Arc<Query>> {
    let query_str = r#"
(comparison_operator) @comparison
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile comparison query for Python")?;
    Ok(Arc::new(query))
}

pub fn compile_class_def_query() -> Result<Arc<Query>> {
    let query_str = r#"
(class_definition
  name: (identifier) @class_name
  body: (block) @class_body) @class_def
"#;
    let query = Query::new(&python_lang(), query_str)
        .with_context(|| "failed to compile class_definition query for Python")?;
    Ok(Arc::new(query))
}
