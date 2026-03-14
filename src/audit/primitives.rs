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

#[derive(Debug, Clone)]
pub struct StructuralMatch {
    pub name: String,
    pub child_count: usize,
    pub line: u32,
    pub column: u32,
    pub snippet: String,
}

pub fn compile_impl_block_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(impl_item
  type: (_) @type_name
  body: (declaration_list) @body) @impl_block
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile impl block query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_struct_fields_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(struct_item
  name: (type_identifier) @struct_name
  body: (field_declaration_list) @fields) @struct_def
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile struct fields query for {language}"))?;
    Ok(Arc::new(query))
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

pub fn find_large_impl_blocks(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    min_methods: usize,
) -> Vec<StructuralMatch> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);
    let mut results = Vec::new();

    let type_name_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "type_name")
        .expect("query must have @type_name capture");
    let body_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "body")
        .expect("query must have @body capture");
    let impl_block_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "impl_block")
        .expect("query must have @impl_block capture");

    while let Some(m) = matches.next() {
        let type_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == type_name_idx)
            .map(|c| c.node);
        let body_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == body_idx)
            .map(|c| c.node);
        let impl_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == impl_block_idx)
            .map(|c| c.node);

        if let (Some(type_node), Some(body_node), Some(impl_node)) =
            (type_node, body_node, impl_node)
        {
            let method_count = (0..body_node.named_child_count())
                .filter_map(|i| body_node.named_child(i))
                .filter(|child| child.kind() == "function_item")
                .count();

            if method_count >= min_methods {
                let name = type_node
                    .utf8_text(source)
                    .unwrap_or("")
                    .to_string();
                let start = impl_node.start_position();
                results.push(StructuralMatch {
                    name,
                    child_count: method_count,
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    snippet: extract_snippet(source, impl_node, 3),
                });
            }
        }
    }

    results
}

pub fn find_large_structs(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    min_fields: usize,
) -> Vec<StructuralMatch> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);
    let mut results = Vec::new();

    let struct_name_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "struct_name")
        .expect("query must have @struct_name capture");
    let fields_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "fields")
        .expect("query must have @fields capture");
    let struct_def_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "struct_def")
        .expect("query must have @struct_def capture");

    while let Some(m) = matches.next() {
        let name_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == struct_name_idx)
            .map(|c| c.node);
        let fields_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == fields_idx)
            .map(|c| c.node);
        let struct_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == struct_def_idx)
            .map(|c| c.node);

        if let (Some(name_node), Some(fields_node), Some(struct_node)) =
            (name_node, fields_node, struct_node)
        {
            let field_count = (0..fields_node.named_child_count())
                .filter_map(|i| fields_node.named_child(i))
                .filter(|child| child.kind() == "field_declaration")
                .count();

            if field_count >= min_fields {
                let name = name_node
                    .utf8_text(source)
                    .unwrap_or("")
                    .to_string();
                let start = struct_node.start_position();
                results.push(StructuralMatch {
                    name,
                    child_count: field_count,
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    snippet: extract_snippet(source, struct_node, 3),
                });
            }
        }
    }

    results
}

pub fn compile_field_declaration_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(field_declaration
  name: (field_identifier) @field_name
  type: (_) @field_type) @field
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile field declaration query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_parameter_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(parameter
  pattern: (_) @param_name
  type: (_) @param_type) @param
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile parameter query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_generic_type_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(generic_type
  type: (_) @outer_type
  type_arguments: (type_arguments
    (generic_type
      type: (_) @inner_type))) @generic
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile generic type query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_scoped_call_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(call_expression
  function: (scoped_identifier) @scoped_fn) @call
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile scoped call query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_function_item_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
(function_item
  name: (identifier) @fn_name
  body: (block) @fn_body) @fn_def
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile function item query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query_str = r#"
[(integer_literal) @number (float_literal) @number]
"#;
    let query = Query::new(&ts_lang, query_str)
        .with_context(|| format!("failed to compile numeric literal query for {language}"))?;
    Ok(Arc::new(query))
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

    // --- StructuralMatch tests ---

    fn gen_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    fn method_{}(&self) {{}}", i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn gen_fields(n: usize) -> String {
        (0..n)
            .map(|i| format!("    field_{}: i32,", i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn large_impl_above_threshold() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(5));
        let (tree, source) = parse_rust(&src);
        let query = compile_impl_block_query(Language::Rust).unwrap();
        let matches = find_large_impl_blocks(&tree, &source, &query, 5);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "Foo");
        assert_eq!(matches[0].child_count, 5);
    }

    #[test]
    fn small_impl_below_threshold() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(2));
        let (tree, source) = parse_rust(&src);
        let query = compile_impl_block_query(Language::Rust).unwrap();
        let matches = find_large_impl_blocks(&tree, &source, &query, 5);
        assert!(matches.is_empty());
    }

    #[test]
    fn impl_mixed_items_only_counts_methods() {
        // 3 methods + 2 const items = 5 named children, but only 3 methods
        let src = r#"
struct Foo;
impl Foo {
    const A: i32 = 1;
    const B: i32 = 2;
    fn method_a(&self) {}
    fn method_b(&self) {}
    fn method_c(&self) {}
}
"#;
        let (tree, source) = parse_rust(src);
        let query = compile_impl_block_query(Language::Rust).unwrap();
        let matches = find_large_impl_blocks(&tree, &source, &query, 4);
        assert!(matches.is_empty());
        let matches = find_large_impl_blocks(&tree, &source, &query, 3);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].child_count, 3);
    }

    #[test]
    fn large_struct_above_threshold() {
        let src = format!("struct BigStruct {{\n{}\n}}", gen_fields(6));
        let (tree, source) = parse_rust(&src);
        let query = compile_struct_fields_query(Language::Rust).unwrap();
        let matches = find_large_structs(&tree, &source, &query, 5);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "BigStruct");
        assert_eq!(matches[0].child_count, 6);
    }

    #[test]
    fn small_struct_below_threshold() {
        let src = format!("struct SmallStruct {{\n{}\n}}", gen_fields(2));
        let (tree, source) = parse_rust(&src);
        let query = compile_struct_fields_query(Language::Rust).unwrap();
        let matches = find_large_structs(&tree, &source, &query, 5);
        assert!(matches.is_empty());
    }

    #[test]
    fn tuple_struct_not_matched() {
        let src = "struct Wrapper(i32, String, Vec<u8>, bool, f64, usize);";
        let (tree, source) = parse_rust(src);
        let query = compile_struct_fields_query(Language::Rust).unwrap();
        let matches = find_large_structs(&tree, &source, &query, 1);
        assert!(matches.is_empty());
    }

    #[test]
    fn correct_type_name_extraction() {
        let src = format!("struct Bar;\nimpl Bar {{\n{}\n}}", gen_methods(3));
        let (tree, source) = parse_rust(&src);
        let query = compile_impl_block_query(Language::Rust).unwrap();
        let matches = find_large_impl_blocks(&tree, &source, &query, 3);
        assert_eq!(matches[0].name, "Bar");
    }

    #[test]
    fn trait_impl_matched() {
        let methods = gen_methods(4);
        let src = format!(
            "struct Foo;\ntrait MyTrait {{}}\nimpl MyTrait for Foo {{\n{}\n}}",
            methods
        );
        let (tree, source) = parse_rust(&src);
        let query = compile_impl_block_query(Language::Rust).unwrap();
        let matches = find_large_impl_blocks(&tree, &source, &query, 4);
        assert_eq!(matches.len(), 1);
        // type capture is the trait name in `impl Trait for Type` — tree-sitter captures `MyTrait`
        // as the `type` field since it's the first type node
    }
}
