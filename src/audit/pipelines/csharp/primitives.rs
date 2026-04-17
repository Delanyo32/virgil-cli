// PERMANENT RUST EXCEPTION: This file is kept only for the 3 taint-based
// security pipelines (csharp_ssrf, sql_injection, xxe) that cannot be
// migrated to JSON. All non-taint primitives were removed when the 15
// tech-debt and code-style pipelines were migrated to JSON.

use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Query;

use crate::language::Language;

pub use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub fn compile_invocation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(invocation_expression
  function: (_) @fn_expr
  arguments: (argument_list) @args) @invocation
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile invocation_expression query for C#")?;
    Ok(Arc::new(query))
}

pub fn compile_object_creation_query() -> Result<Arc<Query>> {
    let query_str = r#"
(object_creation_expression
  type: (_) @type_name
  arguments: (argument_list) @args) @creation
"#;
    let query = Query::new(&csharp_lang(), query_str)
        .with_context(|| "failed to compile object_creation_expression query for C#")?;
    Ok(Arc::new(query))
}
