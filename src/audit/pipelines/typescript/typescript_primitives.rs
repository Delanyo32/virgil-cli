use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn ts_lang(language: Language) -> tree_sitter::Language {
    language.tree_sitter_language()
}

pub fn compile_predefined_type_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"(predefined_type) @predefined"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile predefined_type query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_as_expression_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"(as_expression) @as_expr"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile as_expression query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_interface_declaration_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(interface_declaration
  name: (type_identifier) @name
  body: (interface_body) @body) @decl
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile interface_declaration query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_generic_type_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(generic_type
  name: (type_identifier) @name
  type_arguments: (type_arguments) @args) @generic
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile generic_type query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_enum_declaration_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(enum_declaration
  name: (identifier) @name
  body: (enum_body) @body) @decl
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile enum_declaration query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_function_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
[
  (function_declaration
    parameters: (formal_parameters) @params) @func
  (arrow_function
    parameters: (formal_parameters) @params) @func
  (method_definition
    parameters: (formal_parameters) @params) @func
]
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile function query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_subscript_expression_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(subscript_expression
  object: (_) @obj
  index: (_) @idx) @sub
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile subscript_expression query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn compile_type_parameter_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(type_parameter
  name: (type_identifier) @name) @param
"#;
    let query = Query::new(&ts_lang(language), query_str)
        .with_context(|| "failed to compile type_parameter query for TypeScript")?;
    Ok(Arc::new(query))
}

pub fn is_test_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains(".test.") || lower.contains(".spec.") || lower.contains("__tests__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_ts(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
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
    fn predefined_type_compiles_and_matches() {
        let src = "let x: any = 1;";
        let (tree, source) = parse_ts(src);
        let query = compile_predefined_type_query(Language::TypeScript).unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn as_expression_compiles_and_matches() {
        let src = "let x = y as string;";
        let (tree, source) = parse_ts(src);
        let query = compile_as_expression_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn interface_declaration_compiles_and_matches() {
        let src = "interface Foo { bar: string; }";
        let (tree, source) = parse_ts(src);
        let query = compile_interface_declaration_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn generic_type_compiles_and_matches() {
        let src = "let x: Record<string, any>;";
        let (tree, source) = parse_ts(src);
        let query = compile_generic_type_query(Language::TypeScript).unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn enum_declaration_compiles_and_matches() {
        let src = "enum Color { Red, Green, Blue }";
        let (tree, source) = parse_ts(src);
        let query = compile_enum_declaration_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_query_compiles_and_matches() {
        let src = "function foo(a: number): string { return ''; }";
        let (tree, source) = parse_ts(src);
        let query = compile_function_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn subscript_expression_compiles_and_matches() {
        let src = "let x = arr[0];";
        let (tree, source) = parse_ts(src);
        let query = compile_subscript_expression_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn type_parameter_compiles_and_matches() {
        let src = "function foo<T>(x: T): T { return x; }";
        let (tree, source) = parse_ts(src);
        let query = compile_type_parameter_query(Language::TypeScript).unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn tsx_queries_compile() {
        compile_predefined_type_query(Language::Tsx).unwrap();
        compile_as_expression_query(Language::Tsx).unwrap();
        compile_interface_declaration_query(Language::Tsx).unwrap();
        compile_generic_type_query(Language::Tsx).unwrap();
        compile_enum_declaration_query(Language::Tsx).unwrap();
        compile_function_query(Language::Tsx).unwrap();
        compile_subscript_expression_query(Language::Tsx).unwrap();
        compile_type_parameter_query(Language::Tsx).unwrap();
    }

    #[test]
    fn is_test_file_detection() {
        assert!(is_test_file("src/foo.test.ts"));
        assert!(is_test_file("src/foo.spec.tsx"));
        assert!(is_test_file("src/__tests__/foo.ts"));
        assert!(!is_test_file("src/foo.ts"));
        assert!(!is_test_file("src/utils.ts"));
    }
}
