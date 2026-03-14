use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;

#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub name: String,
    pub text: String,
    pub line: u32,
    pub column: u32,
}

pub fn compile_method_call_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)) @call
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile method call query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_macro_invocation_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(macro_invocation
  macro: (identifier) @macro_name) @invocation
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile macro invocation query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn find_method_calls(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    target_names: &[&str],
) -> Vec<PatternMatch> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);
    let mut results = Vec::new();

    let name_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "method_name")
        .expect("query must have @method_name capture");
    let call_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "call")
        .expect("query must have @call capture");

    while let Some(m) = matches.next() {
        let name_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == name_idx)
            .map(|c| c.node);
        let call_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == call_idx)
            .map(|c| c.node);

        if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
            let method_name = name_node
                .utf8_text(source)
                .unwrap_or("")
                .to_string();

            if target_names.contains(&method_name.as_str()) {
                let text = call_node
                    .utf8_text(source)
                    .unwrap_or("")
                    .to_string();
                let start = call_node.start_position();
                results.push(PatternMatch {
                    name: method_name,
                    text,
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                });
            }
        }
    }

    results
}

pub fn find_macro_invocations(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    target_names: &[&str],
) -> Vec<PatternMatch> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);
    let mut results = Vec::new();

    let name_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "macro_name")
        .expect("query must have @macro_name capture");
    let invocation_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "invocation")
        .expect("query must have @invocation capture");

    while let Some(m) = matches.next() {
        let name_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == name_idx)
            .map(|c| c.node);
        let invocation_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == invocation_idx)
            .map(|c| c.node);

        if let (Some(name_node), Some(invocation_node)) = (name_node, invocation_node) {
            let macro_name = name_node
                .utf8_text(source)
                .unwrap_or("")
                .to_string();

            if target_names.contains(&macro_name.as_str()) {
                let text = invocation_node
                    .utf8_text(source)
                    .unwrap_or("")
                    .to_string();
                let start = invocation_node.start_position();
                results.push(PatternMatch {
                    name: macro_name,
                    text,
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                });
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.as_bytes().to_vec())
    }

    #[test]
    fn find_unwrap_calls() {
        let src = r#"fn main() { let x = Some(1).unwrap(); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_method_call_query(Language::Rust).unwrap();
        let matches = find_method_calls(&tree, &source, &query, &["unwrap"]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "unwrap");
        assert!(matches[0].text.contains("unwrap()"));
    }

    #[test]
    fn find_expect_calls() {
        let src = r#"fn main() { let x = Some(1).expect("msg"); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_method_call_query(Language::Rust).unwrap();
        let matches = find_method_calls(&tree, &source, &query, &["expect"]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "expect");
    }

    #[test]
    fn find_panic_macro() {
        let src = r#"fn main() { panic!("oops"); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_macro_invocation_query(Language::Rust).unwrap();
        let matches = find_macro_invocations(&tree, &source, &query, &["panic"]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "panic");
    }

    #[test]
    fn ignores_non_matching_methods() {
        let src = r#"fn main() { let v = vec![1,2,3]; let n = v.len(); v.iter(); let x = Some(1).unwrap_or(0); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_method_call_query(Language::Rust).unwrap();
        let matches = find_method_calls(&tree, &source, &query, &["unwrap", "expect"]);
        assert!(matches.is_empty());
    }

    #[test]
    fn finds_chained_unwrap() {
        let src = r#"fn main() { let x = Some(Some(1)).unwrap().unwrap(); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_method_call_query(Language::Rust).unwrap();
        let matches = find_method_calls(&tree, &source, &query, &["unwrap"]);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn finds_multiple_target_names() {
        let src = r#"fn main() { let a = Some(1).unwrap(); let b = Some(2).expect("x"); }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_method_call_query(Language::Rust).unwrap();
        let matches = find_method_calls(&tree, &source, &query, &["unwrap", "expect"]);
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().any(|m| m.name == "unwrap"));
        assert!(matches.iter().any(|m| m.name == "expect"));
    }

    #[test]
    fn ignores_non_matching_macros() {
        let src = r#"fn main() { println!("hi"); let v = vec![1,2,3]; }"#;
        let (tree, source) = parse_rust(src);
        let query = compile_macro_invocation_query(Language::Rust).unwrap();
        let matches = find_macro_invocations(
            &tree,
            &source,
            &query,
            &["panic", "todo", "unimplemented", "unreachable"],
        );
        assert!(matches.is_empty());
    }
}
