use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_default_parameter_query, extract_snippet, find_capture_index, node_text,
};

const MUTABLE_KINDS: &[&str] = &["list", "dictionary", "set"];

pub struct MutableDefaultArgsPipeline {
    default_param_query: Arc<Query>,
}

impl MutableDefaultArgsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            default_param_query: compile_default_parameter_query()?,
        })
    }
}

impl Pipeline for MutableDefaultArgsPipeline {
    fn name(&self) -> &str {
        "mutable_default_args"
    }

    fn description(&self) -> &str {
        "Detects mutable default arguments (list, dict, set) in function parameters"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.default_param_query, tree.root_node(), source);

        let param_idx = find_capture_index(&self.default_param_query, "default_param");

        while let Some(m) = matches.next() {
            let param_cap = m.captures.iter().find(|c| c.index as usize == param_idx);

            if let Some(param_cap) = param_cap {
                let node = param_cap.node;

                // Find the value child — it's the last named child for default_parameter,
                // and for typed_default_parameter it's also named "value"
                let value_node = node.child_by_field_name("value");

                if let Some(value) = value_node
                    && MUTABLE_KINDS.contains(&value.kind())
                {
                    let start = node.start_position();
                    let param_text = node_text(node, source);
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "mutable_default_arg".to_string(),
                            message: format!(
                                "mutable default argument `{param_text}` — use `None` and initialize inside the function"
                            ),
                            snippet: extract_snippet(source, node, 1),
                        });
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MutableDefaultArgsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_list_default() {
        let src = "def foo(items=[]):\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_default_arg");
    }

    #[test]
    fn detects_dict_default() {
        let src = "def foo(data={}):\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_typed_mutable_default() {
        let src = "def foo(items: list = []):\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_none_default() {
        let src = "def foo(items=None):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_scalar_default() {
        let src = "def foo(count=0):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
