use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_function_def_query, find_capture_index, node_text};

pub struct DuplicateLogicPipeline {
    fn_query: Arc<Query>,
}

impl DuplicateLogicPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
        })
    }

    fn normalize_params(params_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut param_types = Vec::new();

        for i in 0..params_node.named_child_count() {
            if let Some(child) = params_node.named_child(i) {
                match child.kind() {
                    "identifier" => {
                        let name = node_text(child, source);
                        if name == "self" || name == "cls" {
                            param_types.push(name.to_string());
                        } else {
                            // Untyped param — use placeholder
                            param_types.push("_".to_string());
                        }
                    }
                    "typed_parameter" => {
                        // Use the type annotation
                        if let Some(type_node) = child.child_by_field_name("type") {
                            param_types.push(node_text(type_node, source).to_string());
                        } else {
                            param_types.push("_".to_string());
                        }
                    }
                    "default_parameter" => {
                        param_types.push("_=".to_string());
                    }
                    "typed_default_parameter" => {
                        if let Some(type_node) = child.child_by_field_name("type") {
                            param_types
                                .push(format!("{}=", node_text(type_node, source)));
                        } else {
                            param_types.push("_=".to_string());
                        }
                    }
                    "list_splat_pattern" => {
                        param_types.push("*args".to_string());
                    }
                    "dictionary_splat_pattern" => {
                        param_types.push("**kwargs".to_string());
                    }
                    _ => {}
                }
            }
        }

        // Filter out trivial signatures: empty or sole "self"
        let non_self: Vec<&String> = param_types
            .iter()
            .filter(|p| *p != "self" && *p != "cls")
            .collect();

        if non_self.is_empty() {
            return None;
        }

        Some(param_types.join(", "))
    }
}

impl Pipeline for DuplicateLogicPipeline {
    fn name(&self) -> &str {
        "duplicate_logic"
    }

    fn description(&self) -> &str {
        "Detects functions with identical parameter signatures (potential copy-paste)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let params_idx = find_capture_index(&self.fn_query, "params");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        // signature -> vec of (fn_name, line, column)
        let mut sig_map: HashMap<String, Vec<(String, u32, u32)>> = HashMap::new();

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(params_node), Some(def_node)) =
                (name_node, params_node, def_node)
            {
                let fn_name = node_text(name_node, source).to_string();

                if let Some(sig) = Self::normalize_params(params_node, source) {
                    let start = def_node.start_position();
                    sig_map.entry(sig).or_default().push((
                        fn_name,
                        start.row as u32 + 1,
                        start.column as u32 + 1,
                    ));
                }
            }
        }

        for (sig, funcs) in &sig_map {
            if funcs.len() >= 2 {
                let names: Vec<&str> = funcs.iter().map(|(n, _, _)| n.as_str()).collect();
                for (fn_name, line, column) in funcs {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: *line,
                        column: *column,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "similar_function_signature".to_string(),
                        message: format!(
                            "function `{fn_name}` shares signature `({sig})` with: {}",
                            names
                                .iter()
                                .filter(|n| **n != fn_name.as_str())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        snippet: format!("def {fn_name}({sig})"),
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
        let pipeline = DuplicateLogicPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_duplicate_signatures() {
        let src = "\
def process_user(name, age, email):
    pass

def process_order(name, age, email):
    pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].pattern, "similar_function_signature");
    }

    #[test]
    fn clean_different_signatures() {
        let src = "\
def foo(x, y):
    pass

def bar(a, b, c):
    pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_trivial_self_only() {
        let src = "\
class Foo:
    def method_a(self):
        pass
    def method_b(self):
        pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
