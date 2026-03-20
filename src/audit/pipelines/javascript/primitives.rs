use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub fn compile_variable_declaration_query() -> Result<Arc<Query>> {
    let query_str = r#"
(variable_declaration) @var_decl
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile variable_declaration query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression) @binary
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile binary_expression query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_call_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (member_expression
    object: (_) @obj
    property: (property_identifier) @method)
  arguments: (arguments) @args) @call
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile call_expression query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_function_query() -> Result<Arc<Query>> {
    let query_str = r#"
[
  (function_declaration
    parameters: (formal_parameters) @params
    body: (statement_block) @body) @func
  (arrow_function
    parameters: (formal_parameters) @params
    body: (_) @body) @func
  (function_expression
    parameters: (formal_parameters) @params
    body: (statement_block) @body) @func
]
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile function query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_assignment_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_expression
  left: (_) @lhs
  right: (_) @rhs) @assign
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile assignment_expression query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_numeric_literal_query() -> Result<Arc<Query>> {
    let query_str = r#"
(number) @number
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile numeric literal query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_spread_in_object_query() -> Result<Arc<Query>> {
    let query_str = r#"
(object
  (spread_element
    (_) @target) @spread) @obj
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile spread_in_object query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_if_statement_query() -> Result<Arc<Query>> {
    let query_str = r#"
(if_statement
  condition: (parenthesized_expression) @condition) @if_stmt
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile if_statement query for JavaScript")?;
    Ok(Arc::new(query))
}

pub fn compile_member_expression_query() -> Result<Arc<Query>> {
    let query_str = r#"
(member_expression
  object: (_) @object
  property: (property_identifier) @prop) @member
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile member_expression query for JavaScript")?;
    Ok(Arc::new(query))
}

// --- Parameterized security query compilers (work with both JS and TS grammars) ---

pub fn compile_direct_call_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args) @call
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile direct_call query")?;
    Ok(Arc::new(query))
}

pub fn compile_method_call_security_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(call_expression
  function: (member_expression
    object: (_) @obj
    property: (property_identifier) @method)
  arguments: (arguments) @args) @call
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile method_call_security query")?;
    Ok(Arc::new(query))
}

pub fn compile_new_expression_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(new_expression
  constructor: (identifier) @constructor
  arguments: (arguments) @args) @new_expr
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile new_expression query")?;
    Ok(Arc::new(query))
}

pub fn compile_property_assignment_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_expression
  left: (member_expression
    object: (_) @obj
    property: (property_identifier) @prop)
  right: (_) @value) @assign
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile property_assignment query")?;
    Ok(Arc::new(query))
}

pub fn compile_subscript_assignment_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(assignment_expression
  left: (subscript_expression
    object: (_) @target
    index: (_) @key)
  right: (_) @value) @assign
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile subscript_assignment query")?;
    Ok(Arc::new(query))
}

pub fn compile_regex_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"(regex) @regex"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile regex query")?;
    Ok(Arc::new(query))
}

pub fn compile_binary_expression_security_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(binary_expression
  left: (_) @left
  right: (_) @right) @binary
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile binary_expression_security query")?;
    Ok(Arc::new(query))
}

pub fn compile_for_in_query(language: Language) -> Result<Arc<Query>> {
    let query_str = r#"
(for_in_statement
  left: (_) @var
  right: (_) @iterable
  body: (_) @body) @for_in
"#;
    let query = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile for_in query")?;
    Ok(Arc::new(query))
}

/// Check if a node is a safe literal (string without interpolation, or number)
pub fn is_safe_literal(node: tree_sitter::Node, _source: &[u8]) -> bool {
    match node.kind() {
        "string" => true,
        "template_string" => {
            // Safe only if no template substitutions
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i)
                    && child.kind() == "template_substitution" {
                        return false;
                    }
            }
            true
        }
        "number" | "true" | "false" | "null" | "undefined" => true,
        _ => false,
    }
}

/// Detect nested quantifiers in regex text: (x+)+, (a*)*, ([a-z]+)* etc.
pub fn has_nested_quantifier(regex_text: &str) -> bool {
    let mut depth = 0;
    let mut prev_quantifier_at_depth = [false; 64];
    let chars: Vec<char> = regex_text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 2; // skip escaped char
                continue;
            }
            '(' => {
                depth += 1;
                if depth < prev_quantifier_at_depth.len() {
                    prev_quantifier_at_depth[depth] = false;
                }
            }
            ')' => {
                depth = depth.saturating_sub(1);
                // Check if a quantifier follows this group
                if i + 1 < chars.len() && matches!(chars[i + 1], '+' | '*' | '?') {
                    // If the group contained a quantifier, we have nested quantifiers
                    if depth + 1 < prev_quantifier_at_depth.len()
                        && prev_quantifier_at_depth[depth + 1]
                    {
                        return true;
                    }
                    if depth < prev_quantifier_at_depth.len() {
                        prev_quantifier_at_depth[depth] = true;
                    }
                }
            }
            '+' | '*' => {
                if depth < prev_quantifier_at_depth.len() {
                    prev_quantifier_at_depth[depth] = true;
                }
            }
            '[' => {
                // Skip character class
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

pub fn compile_function_with_body_query() -> Result<Arc<Query>> {
    let query_str = r#"
[
  (function_declaration
    name: (identifier) @func_name
    body: (statement_block) @func_body) @func

  (lexical_declaration
    (variable_declarator
      name: (identifier) @func_name
      value: (arrow_function
        body: (statement_block) @func_body))) @func

  (method_definition
    name: (property_identifier) @func_name
    body: (statement_block) @func_body) @func
]
"#;
    let query = Query::new(&js_lang(), query_str)
        .with_context(|| "failed to compile function_with_body query for JavaScript")?;
    Ok(Arc::new(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;
    use tree_sitter::QueryCursor;

    fn parse_js(source: &str) -> (tree_sitter::Tree, Vec<u8>) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&js_lang()).unwrap();
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
    fn variable_declaration_compiles_and_matches() {
        let src = "var x = 1;";
        let (tree, source) = parse_js(src);
        let query = compile_variable_declaration_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn variable_declaration_skips_let_const() {
        let src = "let x = 1; const y = 2;";
        let (tree, source) = parse_js(src);
        let query = compile_variable_declaration_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 0);
    }

    #[test]
    fn binary_expression_compiles_and_matches() {
        let src = "x == 1;";
        let (tree, source) = parse_js(src);
        let query = compile_binary_expression_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn call_expression_compiles_and_matches() {
        let src = "console.log('hello');";
        let (tree, source) = parse_js(src);
        let query = compile_call_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_query_compiles_and_matches() {
        let src = "function foo(a, b) { return a + b; }";
        let (tree, source) = parse_js(src);
        let query = compile_function_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn assignment_expression_compiles_and_matches() {
        let src = "x = 1;";
        let (tree, source) = parse_js(src);
        let query = compile_assignment_expression_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn numeric_literal_compiles_and_matches() {
        let src = "let x = 42; let y = 3.14;";
        let (tree, source) = parse_js(src);
        let query = compile_numeric_literal_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 2);
    }

    #[test]
    fn spread_in_object_compiles_and_matches() {
        let src = "let y = { ...obj };";
        let (tree, source) = parse_js(src);
        let query = compile_spread_in_object_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn if_statement_compiles_and_matches() {
        let src = "if (x) { foo(); }";
        let (tree, source) = parse_js(src);
        let query = compile_if_statement_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn member_expression_compiles_and_matches() {
        let src = "let x = a.b.c;";
        let (tree, source) = parse_js(src);
        let query = compile_member_expression_query().unwrap();
        assert!(count_matches(&query, &tree, &source) >= 1);
    }

    #[test]
    fn function_with_body_compiles_and_matches_declaration() {
        let src = "function foo() { return 1; }";
        let (tree, source) = parse_js(src);
        let query = compile_function_with_body_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_with_body_compiles_and_matches_arrow() {
        let src = "const foo = () => { return 1; };";
        let (tree, source) = parse_js(src);
        let query = compile_function_with_body_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }

    #[test]
    fn function_with_body_compiles_and_matches_method() {
        let src = "const obj = { foo() { return 1; } };";
        let (tree, source) = parse_js(src);
        let query = compile_function_with_body_query().unwrap();
        assert_eq!(count_matches(&query, &tree, &source), 1);
    }
}
