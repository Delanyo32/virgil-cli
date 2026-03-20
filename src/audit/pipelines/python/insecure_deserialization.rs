use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const DANGEROUS_DESERIALIZE: &[(&str, &str, &str)] = &[
    ("pickle", "loads", "pickle_deserialize"),
    ("pickle", "load", "pickle_deserialize"),
    ("marshal", "loads", "marshal_deserialize"),
    ("marshal", "load", "marshal_deserialize"),
    ("shelve", "open", "shelve_open"),
    ("yaml", "load", "yaml_unsafe_load"),
];

pub struct InsecureDeserializationPipeline {
    call_query: Arc<Query>,
}

impl InsecureDeserializationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for InsecureDeserializationPipeline {
    fn name(&self) -> &str {
        "insecure_deserialization"
    }

    fn description(&self) -> &str {
        "Detects insecure deserialization: pickle, marshal, shelve, yaml.load with untrusted data"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(call_node)) =
                (fn_node, args_node, call_node)
            {
                if fn_node.kind() != "attribute" {
                    continue;
                }

                let obj = fn_node
                    .child_by_field_name("object")
                    .map(|n| node_text(n, source));
                let attr = fn_node
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source));

                if let (Some(obj_name), Some(attr_name)) = (obj, attr) {
                    // Check yaml.load specifically — yaml.safe_load is fine
                    if obj_name == "yaml" && attr_name == "load" {
                        // Check if Loader=SafeLoader is passed
                        let call_text = node_text(call_node, source);
                        if call_text.contains("SafeLoader") || call_text.contains("safe_load") {
                            continue;
                        }
                    }

                    let matching = DANGEROUS_DESERIALIZE
                        .iter()
                        .find(|(module, method, _)| *module == obj_name && *method == attr_name);

                    if let Some((module, method, pattern)) = matching {
                        // Check that first arg is not a plain string literal
                        if let Some(first_arg) = args_node.named_child(0)
                            && first_arg.kind() == "string"
                            && !has_interpolation(first_arg)
                        {
                            continue;
                        }

                        let start = call_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: pattern.to_string(),
                            message: format!(
                                "`{module}.{method}()` deserializes untrusted data — potential code execution"
                            ),
                            snippet: extract_snippet(source, call_node, 1),
                        });
                    }
                }
            }
        }

        findings
    }
}

fn has_interpolation(node: tree_sitter::Node) -> bool {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i)
            && child.kind() == "interpolation"
        {
            return true;
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InsecureDeserializationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_pickle_loads() {
        let src = "import pickle\npickle.loads(data)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "pickle_deserialize");
    }

    #[test]
    fn detects_marshal_loads() {
        let src = "import marshal\nmarshal.loads(data)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "marshal_deserialize");
    }

    #[test]
    fn detects_shelve_open() {
        let src = "import shelve\nshelve.open(filename)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shelve_open");
    }

    #[test]
    fn ignores_json_loads() {
        let src = "import json\njson.loads(data)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_yaml_safe_load() {
        let src = "import yaml\nyaml.load(data, Loader=yaml.SafeLoader)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
