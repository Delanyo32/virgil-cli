use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_method_call_security_query, extract_snippet, find_capture_index, is_safe_literal,
    node_text,
};

pub struct PathTraversalPipeline {
    method_call_query: Arc<Query>,
}

impl PathTraversalPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

/// Check if a node is inside a function that has parameters (indicates external input)
fn is_in_parameterized_function(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration"
            | "arrow_function"
            | "function_expression"
            | "method_definition" => {
                if let Some(params) = parent.child_by_field_name("parameters") {
                    return params.named_child_count() > 0;
                }
                return false;
            }
            _ => {
                current = parent.parent();
            }
        }
    }
    false
}

impl Pipeline for PathTraversalPipeline {
    fn name(&self) -> &str {
        "path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal: path.join/resolve with user params, fs.readFile/readFileSync with variables"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
        let obj_idx = find_capture_index(&self.method_call_query, "obj");
        let method_idx = find_capture_index(&self.method_call_query, "method");
        let args_idx = find_capture_index(&self.method_call_query, "args");
        let call_idx = find_capture_index(&self.method_call_query, "call");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
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

            if let (Some(obj), Some(method), Some(args), Some(call)) =
                (obj_node, method_node, args_node, call_node)
            {
                let obj_name = node_text(obj, source);
                let method_name = node_text(method, source);

                // path.join / path.resolve with params — only flag inside parameterized functions
                if obj_name == "path"
                    && (method_name == "join" || method_name == "resolve")
                    && args.named_child_count() >= 2
                    && is_in_parameterized_function(call)
                {
                    // Check if any arg is non-literal (could be user input)
                    let has_dynamic_arg = (0..args.named_child_count()).any(|i| {
                        args.named_child(i)
                            .map(|a| !is_safe_literal(a, source))
                            .unwrap_or(false)
                    });
                    if has_dynamic_arg {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: format!("unvalidated_path_{}", method_name),
                            message: format!(
                                "`path.{}()` with dynamic segments — validate path before use",
                                method_name
                            ),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }
                }

                // fs.readFile / fs.readFileSync / fs.writeFile / fs.writeFileSync with variable
                if obj_name == "fs"
                    && matches!(
                        method_name,
                        "readFile" | "readFileSync" | "writeFile" | "writeFileSync"
                    )
                    && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source) {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unvalidated_fs_read".to_string(),
                                message: format!(
                                    "`fs.{}()` with dynamic path — potential path traversal",
                                    method_name
                                ),
                                snippet: extract_snippet(source, call, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::JavaScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PathTraversalPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_path_join_in_function() {
        let src = r#"function serve(userPath) { const p = path.join("/uploads", userPath); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_join");
    }

    #[test]
    fn detects_path_resolve_in_function() {
        let src = r#"function get(dir) { return path.resolve(base, dir); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_resolve");
    }

    #[test]
    fn detects_fs_readfile_with_variable() {
        let src = "fs.readFileSync(filePath);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_fs_read");
    }

    #[test]
    fn ignores_fs_readfile_with_literal() {
        let src = r#"fs.readFileSync("config.json");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_path_join_with_all_literals() {
        let src = r#"function f(x) { path.join("/a", "/b"); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
