// PERMANENT RUST EXCEPTION: This module provides query compilation and utility
// functions for taint-based security pipelines (xss_dom_injection.rs, ssrf.rs).
// These pipelines require FlowsTo/SanitizedBy graph predicates and are not
// expressible in the JSON DSL. Do not migrate or delete this file.

use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

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

/// Check if a node is a safe literal (string without interpolation, or number)
pub fn is_safe_literal(node: tree_sitter::Node, _source: &[u8]) -> bool {
    match node.kind() {
        "string" => true,
        "template_string" => {
            // Safe only if no template substitutions
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i)
                    && child.kind() == "template_substitution"
                {
                    return false;
                }
            }
            true
        }
        "number" | "true" | "false" | "null" | "undefined" => true,
        _ => false,
    }
}
