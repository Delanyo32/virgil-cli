use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index, node_text,
};
use crate::audit::pipelines::helpers::{all_args_are_literals, is_literal_node_cpp};

pub struct CppPathTraversalPipeline {
    fn_query: Arc<Query>,
}

impl CppPathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_definition_query()?,
        })
    }

    fn extract_param_names(fn_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut params = Vec::new();
        if let Some(declarator) = fn_node.child_by_field_name("declarator") {
            Self::walk_for_params(declarator, source, &mut params);
        }
        params
    }

    fn walk_for_params(node: tree_sitter::Node, source: &[u8], params: &mut Vec<String>) {
        if node.kind() == "parameter_declaration" {
            // Get the declarator child which holds the parameter name
            if let Some(decl) = node.child_by_field_name("declarator") {
                let name = Self::find_identifier(decl, source);
                if let Some(name) = name {
                    params.push(name);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_params(child, source, params);
        }
    }

    fn find_identifier(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        if node.kind() == "identifier" {
            return Some(node_text(node, source).to_string());
        }
        if node.kind() == "reference_declarator" || node.kind() == "pointer_declarator" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = Self::find_identifier(child, source) {
                    return Some(name);
                }
            }
        }
        None
    }

    /// Check if a declaration node contains an argument_list where all args are literals.
    fn has_literal_args_only(node: tree_sitter::Node) -> bool {
        // Walk children to find an argument_list
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "argument_list" {
                return all_args_are_literals(child, is_literal_node_cpp);
            }
            // Also check inside declarators (e.g., `ifstream file("literal")`)
            if child.kind() == "init_declarator" || child.kind() == "declarator" {
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "argument_list" {
                        return all_args_are_literals(inner_child, is_literal_node_cpp);
                    }
                }
            }
        }
        false
    }

    fn scan_body_for_ifstream_dynamic(
        &self,
        body: tree_sitter::Node,
        source: &[u8],
        param_names: &[String],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        Self::walk_for_file_streams(
            body,
            source,
            param_names,
            &mut findings,
            file_path,
            self.name(),
        );
        findings
    }

    fn walk_for_file_streams(
        node: tree_sitter::Node,
        source: &[u8],
        param_names: &[String],
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        if node.kind() == "declaration" {
            let text = node_text(node, source);
            let stream_types = ["ifstream", "ofstream", "fstream"];
            for stream_type in &stream_types {
                if text.contains(stream_type) {
                    // Skip if the stream constructor argument is a literal
                    if Self::has_literal_args_only(node) {
                        continue;
                    }

                    // Check if any parameter name appears in the declaration
                    for param in param_names {
                        if text.contains(param.as_str()) {
                            let start = node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "ifstream_dynamic_path".to_string(),
                                message: format!(
                                    "`{stream_type}` opened with dynamic path — validate and canonicalize to prevent directory traversal"
                                ),
                                snippet: extract_snippet(source, node, 1),
                            });
                            return;
                        }
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_file_streams(
                child,
                source,
                param_names,
                findings,
                file_path,
                pipeline_name,
            );
        }
    }

    fn scan_body_for_path_concat(
        &self,
        body: tree_sitter::Node,
        source: &[u8],
        param_names: &[String],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let body_text = node_text(body, source);

        // Check if the function body uses filesystem path concatenation with a parameter
        let has_path_concat = (body_text.contains("fs::path")
            || body_text.contains("filesystem::path"))
            && body_text.contains('/');

        if !has_path_concat {
            return findings;
        }

        // Check if any parameter is involved in path operations
        let mut has_param_in_path = false;
        for param in param_names {
            if body_text.contains(param.as_str()) {
                has_param_in_path = true;
                break;
            }
        }

        if !has_param_in_path {
            return findings;
        }

        // Check if canonical is used
        let has_canonical =
            body_text.contains("canonical") || body_text.contains("weakly_canonical");

        if !has_canonical {
            // Find the path concat node for precise location
            let location = Self::find_path_concat_node(body, source, param_names);
            let (node_for_loc, _) = location.unwrap_or((body, String::new()));

            let start = node_for_loc.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "path_concat_without_canonical".to_string(),
                message:
                    "filesystem path built from parameter without `canonical()` check — risk of directory traversal"
                        .to_string(),
                snippet: extract_snippet(source, node_for_loc, 1),
            });
        }

        findings
    }

    fn find_path_concat_node<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        param_names: &[String],
    ) -> Option<(tree_sitter::Node<'a>, String)> {
        // Look for binary_expression with / operator involving path
        if node.kind() == "binary_expression" {
            let text = node_text(node, source);
            if text.contains('/') {
                for param in param_names {
                    if text.contains(param.as_str()) {
                        return Some((node, param.clone()));
                    }
                }
            }
        }
        // Also check for declaration involving path and a parameter
        if node.kind() == "declaration" {
            let text = node_text(node, source);
            if (text.contains("fs::path") || text.contains("filesystem::path"))
                && text.contains('/')
            {
                for param in param_names {
                    if text.contains(param.as_str()) {
                        return Some((node, param.clone()));
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(result) = Self::find_path_concat_node(child, source, param_names) {
                return Some(result);
            }
        }
        None
    }
}

impl Pipeline for CppPathTraversalPipeline {
    fn name(&self) -> &str {
        "cpp_path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: filesystem path operations with unsanitized input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");
        let body_idx = find_capture_index(&self.fn_query, "fn_body");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_def_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);

            if let (Some(fn_cap), Some(body_cap)) = (fn_cap, body_cap) {
                let param_names = Self::extract_param_names(fn_cap.node, source);
                if param_names.is_empty() {
                    continue;
                }

                // Pattern 1: ifstream/ofstream with dynamic path
                findings.extend(self.scan_body_for_ifstream_dynamic(
                    body_cap.node,
                    source,
                    &param_names,
                    file_path,
                ));

                // Pattern 2: path concatenation without canonical
                findings.extend(self.scan_body_for_path_concat(
                    body_cap.node,
                    source,
                    &param_names,
                    file_path,
                ));
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppPathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_ifstream_param() {
        let src = r#"
void read(const std::string& path) {
    std::ifstream file(path);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "ifstream_dynamic_path")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("ifstream"));
    }

    #[test]
    fn ignores_ifstream_literal() {
        let src = r#"
void read_config() {
    std::ifstream file("config.txt");
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "ifstream_dynamic_path")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_path_concat() {
        let src = r#"
void read(const std::string& name) {
    auto p = std::filesystem::path("/data") / name;
    std::ifstream file(p);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "path_concat_without_canonical")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("canonical"));
    }

    #[test]
    fn no_finding_for_path_with_canonical() {
        let src = r#"
void read(const std::string& name) {
    auto p = std::filesystem::canonical(std::filesystem::path("/data") / name);
    std::ifstream file(p);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "path_concat_without_canonical")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_ofstream_param() {
        let src = r#"
void write_file(const std::string& path) {
    std::ofstream out(path);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "ifstream_dynamic_path")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("ofstream"));
    }

    #[test]
    fn no_finding_for_no_params() {
        let src = r#"
void read_default() {
    std::ifstream file("default.txt");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
void read(const std::string& path) {
    std::ifstream file(path);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "cpp_path_traversal");
    }
}
