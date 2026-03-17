use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub fn compile_function_call_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_call_expression
  function: (name) @fn_name
  arguments: (arguments) @args) @call
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile function_call_expression query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_member_call_query() -> Result<Arc<Query>> {
    let query_str = r#"
(member_call_expression
  object: (_) @object
  name: (name) @method_name
  arguments: (arguments) @args) @call
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile member_call_expression query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_error_suppression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(error_suppression_expression) @suppress
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile error_suppression_expression query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_function_def_query() -> Result<Arc<Query>> {
    let query_str = r#"
(function_definition
  name: (name) @fn_name
  parameters: (formal_parameters) @params
  body: (compound_statement) @fn_body) @fn_def
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile function_definition query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_method_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(method_declaration
  name: (name) @method_name
  parameters: (formal_parameters) @params) @method_decl
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile method_declaration query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_class_decl_query() -> Result<Arc<Query>> {
    let query_str = r#"
(class_declaration
  name: (name) @class_name
  body: (declaration_list) @class_body) @class_decl
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile class_declaration query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_catch_clause_query() -> Result<Arc<Query>> {
    let query_str = r#"
(catch_clause
  type: (type_list) @catch_type
  body: (compound_statement) @catch_body) @catch
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile catch_clause query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_include_require_query() -> Result<Arc<Query>> {
    let query_str = r#"
[
  (include_expression) @include_expr
  (include_once_expression) @include_expr
  (require_expression) @include_expr
  (require_once_expression) @include_expr
]
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile include/require query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_echo_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(echo_statement) @echo
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile echo_statement query for PHP")?;
    Ok(Arc::new(query))
}

pub fn compile_text_node_query() -> Result<Arc<Query>> {
    let query_str = r#"
(text) @html_text
"#;
    let query = Query::new(&php_lang(), query_str)
        .with_context(|| "failed to compile text node query for PHP")?;
    Ok(Arc::new(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_php(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&php_lang()).unwrap();
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
    fn function_call_compiles_and_matches() {
        let src = "<?php\nstrlen('hello');";
        let (tree, source) = parse_php(src);
        let query = compile_function_call_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn member_call_compiles_and_matches() {
        let src = "<?php\n$db->query('SELECT 1');";
        let (tree, source) = parse_php(src);
        let query = compile_member_call_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn error_suppression_compiles_and_matches() {
        let src = "<?php\n@file_get_contents('x');";
        let (tree, source) = parse_php(src);
        let query = compile_error_suppression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_def_compiles_and_matches() {
        let src = "<?php\nfunction hello() { return 1; }";
        let (tree, source) = parse_php(src);
        let query = compile_function_def_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn method_decl_compiles_and_matches() {
        let src = "<?php\nclass Foo { public function bar() {} }";
        let (tree, source) = parse_php(src);
        let query = compile_method_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn class_decl_compiles_and_matches() {
        let src = "<?php\nclass Foo { }";
        let (tree, source) = parse_php(src);
        let query = compile_class_decl_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn catch_clause_compiles_and_matches() {
        let src = "<?php\ntry { } catch (Exception $e) { }";
        let (tree, source) = parse_php(src);
        let query = compile_catch_clause_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn include_require_compiles_and_matches() {
        let src = "<?php\nrequire 'foo.php';\ninclude_once 'bar.php';";
        let (tree, source) = parse_php(src);
        let query = compile_include_require_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn echo_statement_compiles_and_matches() {
        let src = "<?php\necho 'hello';";
        let (tree, source) = parse_php(src);
        let query = compile_echo_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn text_node_compiles_and_matches() {
        let src = "<?php echo 1; ?>\n<h1>Hello</h1>";
        let (tree, source) = parse_php(src);
        let query = compile_text_node_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }
}
