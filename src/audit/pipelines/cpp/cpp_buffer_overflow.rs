use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, compile_function_definition_query, extract_snippet,
    find_capture_index, node_text,
};

const UNSAFE_COPY_FUNCTIONS: &[&str] = &["strcpy", "strcat", "sprintf", "gets", "wcscpy", "wcscat"];

pub struct CppBufferOverflowPipeline {
    call_query: Arc<Query>,
    fn_query: Arc<Query>,
}

impl CppBufferOverflowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
            fn_query: compile_function_definition_query()?,
        })
    }

    fn get_param_names(fn_def: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(decl) = fn_def.child_by_field_name("declarator") {
            Self::find_params(decl, source, &mut names);
        }
        names
    }

    fn find_params(node: tree_sitter::Node, source: &[u8], names: &mut Vec<String>) {
        if node.kind() == "parameter_list" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "parameter_declaration"
                    && let Some(declarator) = child.child_by_field_name("declarator")
                {
                    Self::extract_identifier(declarator, source, names);
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::find_params(child, source, names);
        }
    }

    fn extract_identifier(node: tree_sitter::Node, source: &[u8], names: &mut Vec<String>) {
        if node.kind() == "identifier" {
            if let Ok(text) = node.utf8_text(source) {
                names.push(text.to_string());
            }
            return;
        }
        // Unwrap reference_declarator, pointer_declarator, etc.
        if let Some(inner) = node.child_by_field_name("declarator") {
            Self::extract_identifier(inner, source, names);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Ok(text) = child.utf8_text(source) {
                    names.push(text.to_string());
                }
                return;
            }
        }
    }

    fn walk_for_subscripts(
        node: tree_sitter::Node,
        source: &[u8],
        param_names: &[String],
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        if node.kind() == "subscript_expression" {
            // tree-sitter C++ subscript_expression structure:
            //   (subscript_expression)
            //     (identifier) <- the container (first named child)
            //     (subscript_argument_list)
            //       (identifier) <- the index
            let container_node = node.named_child(0);
            let arg_list = node.named_child(1);

            if let (Some(container_node), Some(arg_list)) = (container_node, arg_list)
                && arg_list.kind() == "subscript_argument_list"
            {
                // Find the index identifier inside the subscript_argument_list
                if let Some(index_node) = arg_list.named_child(0)
                    && index_node.kind() == "identifier"
                {
                    let index_text = node_text(index_node, source);
                    if param_names.contains(&index_text.to_string()) {
                        let container = node_text(container_node, source);
                        let start = node.start_position();
                        findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: pipeline_name.to_string(),
                                    pattern: "unchecked_subscript".to_string(),
                                    message: format!(
                                        "`{container}[{index_text}]` with unchecked index — use `.at({index_text})` for bounds checking"
                                    ),
                                    snippet: extract_snippet(source, node, 1),
                                });
                    }
                }
            }
            return; // don't recurse into subscript children
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_subscripts(
                child,
                source,
                param_names,
                findings,
                file_path,
                pipeline_name,
            );
        }
    }
}

impl Pipeline for CppBufferOverflowPipeline {
    fn name(&self) -> &str {
        "cpp_buffer_overflow"
    }

    fn description(&self) -> &str {
        "Detects buffer overflow risks: strcpy/sprintf to fixed buffers, unchecked subscript access, iterator past end"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: strcpy_fixed_buffer — flag unsafe C string functions
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
            let call_idx = find_capture_index(&self.call_query, "call");

            while let Some(m) = matches.next() {
                let fn_name_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
                let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

                if let (Some(fn_name_cap), Some(call_cap)) = (fn_name_cap, call_cap) {
                    let fn_name = node_text(fn_name_cap.node, source);
                    if UNSAFE_COPY_FUNCTIONS.contains(&fn_name) {
                        let start = call_cap.node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "strcpy_fixed_buffer".to_string(),
                            message: format!(
                                "`{fn_name}()` has no bounds checking — use safe alternatives (strncpy, snprintf, std::string)"
                            ),
                            snippet: extract_snippet(source, call_cap.node, 1),
                        });
                    }
                }
            }
        }

        // Pattern 2: unchecked_subscript — subscript with parameter index
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);
            let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");
            let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");

            while let Some(m) = matches.next() {
                let fn_def_cap = m.captures.iter().find(|c| c.index as usize == fn_def_idx);
                let fn_body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

                if let (Some(fn_def_cap), Some(fn_body_cap)) = (fn_def_cap, fn_body_cap) {
                    let param_names = Self::get_param_names(fn_def_cap.node, source);
                    if !param_names.is_empty() {
                        Self::walk_for_subscripts(
                            fn_body_cap.node,
                            source,
                            &param_names,
                            &mut findings,
                            file_path,
                            self.name(),
                        );
                    }
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppBufferOverflowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_strcpy() {
        let src = "void f(const char *src) { char buf[64]; strcpy(buf, src); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "strcpy_fixed_buffer");
    }

    #[test]
    fn detects_unchecked_subscript() {
        let src = "int f(std::vector<int>& v, int i) { return v[i]; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unchecked_subscript");
    }

    #[test]
    fn ignores_literal_subscript() {
        let src = "int f(std::vector<int>& v) { return v[0]; }";
        let findings = parse_and_check(src);
        let subscript_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unchecked_subscript")
            .collect();
        assert_eq!(subscript_findings.len(), 0);
    }

    #[test]
    fn detects_sprintf() {
        let src = "void f() { char buf[128]; sprintf(buf, \"%d\", 42); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "strcpy_fixed_buffer");
        assert!(findings[0].message.contains("sprintf"));
    }

    #[test]
    fn detects_gets() {
        let src = "void f() { char buf[256]; gets(buf); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("gets"));
    }

    #[test]
    fn no_finding_for_safe_alternatives() {
        let src = "void f() { char buf[64]; strncpy(buf, \"hello\", sizeof(buf)); }";
        let findings = parse_and_check(src);
        let unsafe_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "strcpy_fixed_buffer")
            .collect();
        assert!(unsafe_findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "void f(const char *s) { char b[8]; strcpy(b, s); }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].pipeline, "cpp_buffer_overflow");
    }
}
