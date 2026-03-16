use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
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

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_python(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
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
    fn function_def_compiles_and_matches() {
        let src = "def hello():\n    pass";
        let (tree, source) = parse_python(src);
        let query = compile_function_def_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn numeric_literal_compiles_and_matches() {
        let src = "x = 42\ny = 3.14";
        let (tree, source) = parse_python(src);
        let query = compile_numeric_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn except_clause_compiles_and_matches() {
        let src = "try:\n    pass\nexcept:\n    pass";
        let (tree, source) = parse_python(src);
        let query = compile_except_clause_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn default_parameter_compiles_and_matches() {
        let src = "def f(x=1, y=2):\n    pass";
        let (tree, source) = parse_python(src);
        let query = compile_default_parameter_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn comparison_compiles_and_matches() {
        let src = "x = 1 == 2";
        let (tree, source) = parse_python(src);
        let query = compile_comparison_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn class_def_compiles_and_matches() {
        let src = "class Foo:\n    pass";
        let (tree, source) = parse_python(src);
        let query = compile_class_def_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }
}
