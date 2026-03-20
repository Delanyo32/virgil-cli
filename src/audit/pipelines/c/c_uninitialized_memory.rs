use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, node_text,
};

const SEND_FUNCTIONS: &[&str] = &["send", "write", "fwrite", "sendto"];
const INIT_FUNCTIONS: &[&str] = &["memset", "memcpy", "bzero"];

pub struct CUninitializedMemoryPipeline {
    fn_def_query: Arc<Query>,
}

impl CUninitializedMemoryPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Check if a declaration node declares an array without an initializer.
    /// Returns the array variable name if so.
    fn extract_uninit_array(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        if node.kind() != "declaration" {
            return None;
        }

        if let Some(declarator) = node.child_by_field_name("declarator") {
            // init_declarator means there's an = ... initializer
            if declarator.kind() == "init_declarator" {
                // Has an initializer — not uninitialized
                return None;
            }

            // Check if the declarator contains an array_declarator
            if Self::contains_array_declarator(declarator) {
                return find_identifier_in_declarator(declarator, source);
            }
        }

        None
    }

    /// Recursively check if a declarator contains an array_declarator.
    fn contains_array_declarator(node: tree_sitter::Node) -> bool {
        if node.kind() == "array_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::contains_array_declarator(inner);
        }
        false
    }

    /// Check if a call targets one of the initialization functions and its first arg
    /// matches the given buffer name.
    fn is_init_call_for_buffer(node: tree_sitter::Node, source: &[u8], buffer_name: &str) -> bool {
        if node.kind() != "expression_statement" {
            return false;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression"
                && let Some(func) = child.child_by_field_name("function") {
                    let fn_name = node_text(func, source);
                    if INIT_FUNCTIONS.contains(&fn_name)
                        && let Some(args) = child.child_by_field_name("arguments")
                            && let Some(first_arg) = args.named_child(0) {
                                let arg_text = node_text(first_arg, source);
                                if arg_text == buffer_name {
                                    return true;
                                }
                            }
                }
        }
        false
    }

    /// Check if a call is to a send/write function and its buffer argument matches the name.
    /// Returns the function name if found.
    fn is_send_call_for_buffer<'a>(
        node: tree_sitter::Node<'a>,
        source: &'a [u8],
        buffer_name: &str,
    ) -> Option<&'a str> {
        if node.kind() != "expression_statement" {
            return None;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression"
                && let Some(func) = child.child_by_field_name("function") {
                    let fn_name = node_text(func, source);
                    if SEND_FUNCTIONS.contains(&fn_name)
                        && let Some(args) = child.child_by_field_name("arguments") {
                            let mut args_cursor = args.walk();
                            let named_args: Vec<tree_sitter::Node> =
                                args.named_children(&mut args_cursor).collect();
                            // For send/sendto: second arg is the buffer
                            // For write: second arg is the buffer
                            // For fwrite: first arg is the buffer
                            let buf_arg_idx = if fn_name == "fwrite" { 0 } else { 1 };
                            if let Some(arg_node) = named_args.get(buf_arg_idx) {
                                let arg_text = node_text(*arg_node, source);
                                if arg_text == buffer_name {
                                    return Some(fn_name);
                                }
                            }
                        }
                }
        }
        None
    }

    /// Scan a function body for uninitialized buffers passed to send/write functions.
    fn scan_body_for_issues(
        body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut uninit_buffers: Vec<String> = Vec::new();

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            // Track array declarations without initializers
            if child.kind() == "declaration" {
                if let Some(buf_name) = Self::extract_uninit_array(child, source) {
                    uninit_buffers.push(buf_name);
                }
                continue;
            }

            // Check if an initialization function targets one of our tracked buffers
            if !uninit_buffers.is_empty() {
                let mut initialized = Vec::new();
                for buf in &uninit_buffers {
                    if Self::is_init_call_for_buffer(child, source, buf) {
                        initialized.push(buf.clone());
                    }
                }
                for buf in &initialized {
                    uninit_buffers.retain(|b| b != buf);
                }
            }

            // Check if a send/write function references an uninitialized buffer
            if !uninit_buffers.is_empty() {
                for buf in &uninit_buffers {
                    if let Some(fn_name) = Self::is_send_call_for_buffer(child, source, buf) {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "uninitialized_buffer_sent".to_string(),
                            message: format!(
                                "buffer `{buf}` may be sent uninitialized via `{fn_name}()` — could leak sensitive data"
                            ),
                            snippet: extract_snippet(source, child, 1),
                        });
                    }
                }
            }
        }

        findings
    }
}

impl Pipeline for CUninitializedMemoryPipeline {
    fn name(&self) -> &str {
        "c_uninitialized_memory"
    }

    fn description(&self) -> &str {
        "Detects uninitialized memory disclosure: buffers sent/written without initialization"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

            if let Some(body_cap) = body_cap {
                let mut body_findings =
                    Self::scan_body_for_issues(body_cap.node, source, file_path, self.name());
                findings.append(&mut body_findings);
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
        let pipeline = CUninitializedMemoryPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_uninitialized_send() {
        let src = r#"
void f(int fd) {
    char buf[256];
    send(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "uninitialized_buffer_sent");
        assert!(findings[0].message.contains("buf"));
        assert!(findings[0].message.contains("send"));
    }

    #[test]
    fn ignores_initialized_send() {
        let src = r#"
void f(int fd) {
    char buf[256];
    memset(buf, 0, sizeof(buf));
    send(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn ignores_zero_initialized() {
        let src = r#"
void f(int fd) {
    char buf[256] = {0};
    send(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }
}
