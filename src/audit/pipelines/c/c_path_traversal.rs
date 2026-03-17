use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

const FILE_OPEN_FUNCTIONS: &[&str] = &["fopen", "open", "openat"];

pub struct CPathTraversalPipeline {
    call_query: Arc<Query>,
}

impl CPathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    /// Check whether the enclosing function has any parameters.
    fn enclosing_function_has_params(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = Some(node);
        while let Some(n) = current {
            if n.kind() == "function_definition" {
                if let Some(declarator) = n.child_by_field_name("declarator") {
                    return Self::declarator_has_params(declarator, source);
                }
                return false;
            }
            current = n.parent();
        }
        false
    }

    /// Recursively check if a declarator (possibly nested via function_declarator/pointer_declarator)
    /// has a non-empty parameter_list.
    fn declarator_has_params(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "function_declarator" {
            if let Some(params) = node.child_by_field_name("parameters") {
                // parameter_list with at least one named child that is not just "void"
                let mut cursor = params.walk();
                let named: Vec<tree_sitter::Node> = params.named_children(&mut cursor).collect();
                if named.is_empty() {
                    return false;
                }
                // Check for the special case of (void) — single parameter_declaration with type "void"
                if named.len() == 1 && named[0].kind() == "parameter_declaration" {
                    let text = node_text(named[0], source).trim().to_string();
                    if text == "void" {
                        return false;
                    }
                }
                return true;
            }
        }
        // Drill into nested declarators (pointer_declarator, parenthesized_declarator, etc.)
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::declarator_has_params(inner, source);
        }
        false
    }
}

impl Pipeline for CPathTraversalPipeline {
    fn name(&self) -> &str {
        "c_path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: fopen/open with dynamically constructed paths from parameters"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");
        let args_idx = find_capture_index(&self.call_query, "args");

        while let Some(m) = matches.next() {
            let fn_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);

            if let (Some(fn_cap), Some(call_cap), Some(args_cap)) = (fn_cap, call_cap, args_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                if !FILE_OPEN_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                // Get the first named argument (the path)
                let args_node = args_cap.node;
                let mut walker = args_node.walk();
                let named_args: Vec<tree_sitter::Node> =
                    args_node.named_children(&mut walker).collect();

                if let Some(first_arg) = named_args.first() {
                    // If the path is a string literal, it's safe
                    if first_arg.kind() == "string_literal"
                        || first_arg.kind() == "concatenated_string"
                    {
                        continue;
                    }

                    // The path is dynamic — check if the enclosing function has parameters
                    // (meaning the path could be externally controlled)
                    if !Self::enclosing_function_has_params(call_cap.node, source) {
                        continue;
                    }

                    let path_var = node_text(*first_arg, source);
                    let start = call_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "fopen_dynamic_path".to_string(),
                        message: format!(
                            "`{fn_name}()` with dynamic path `{path_var}` — validate and canonicalize path to prevent traversal"
                        ),
                        snippet: extract_snippet(source, call_cap.node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CPathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_fopen_param_path() {
        let src = r#"
void read_file(const char *path) {
    FILE *fp = fopen(path, "r");
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "fopen_dynamic_path");
        assert!(findings[0].message.contains("fopen"));
        assert!(findings[0].message.contains("path"));
    }

    #[test]
    fn ignores_fopen_literal() {
        let src = r#"
void read_config() {
    FILE *fp = fopen("/etc/config.txt", "r");
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_open_param_path() {
        let src = r#"
void open_file(const char *name) {
    int fd = open(name, O_RDONLY);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "fopen_dynamic_path");
        assert!(findings[0].message.contains("open"));
        assert!(findings[0].message.contains("name"));
    }
}
