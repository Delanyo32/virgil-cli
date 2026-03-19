use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

pub struct MissingCancellationTokenPipeline {
    method_query: Arc<Query>,
}

impl MissingCancellationTokenPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_decl_query()?,
        })
    }
}

impl Pipeline for MissingCancellationTokenPipeline {
    fn name(&self) -> &str {
        "missing_cancellation_token"
    }

    fn description(&self) -> &str {
        "Detects async methods without a CancellationToken parameter"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.method_query, "method_name");
        let method_decl_idx = find_capture_index(&self.method_query, "method_decl");
        let params_idx = find_capture_index(&self.method_query, "params");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_decl_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(decl_node), Some(params_node)) =
                (name_node, decl_node, params_node)
            {
                // Only check async methods
                if !has_modifier(decl_node, source, "async") {
                    continue;
                }

                let method_name = node_text(name_node, source);

                // Check if any parameter is CancellationToken
                let has_ct = has_cancellation_token_param(params_node, source);
                if !has_ct {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "no_cancellation_token".to_string(),
                        message: format!(
                            "async method `{method_name}` has no CancellationToken parameter \u{2014} callers cannot cancel the operation"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        findings
    }
}

fn has_cancellation_token_param(params_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_text = node_text(type_node, source);
                if type_text == "CancellationToken" {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingCancellationTokenPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_async_without_ct() {
        let src = r#"
class Foo {
    public async Task DoWorkAsync() {
        await Task.Delay(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "no_cancellation_token");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn clean_async_with_ct() {
        let src = r#"
class Foo {
    public async Task DoWorkAsync(CancellationToken cancellationToken) {
        await Task.Delay(1000, cancellationToken);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_sync_methods() {
        let src = r#"
class Foo {
    public void DoWork() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "class Foo { async Task M() { } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pipeline, "missing_cancellation_token");
    }
}
