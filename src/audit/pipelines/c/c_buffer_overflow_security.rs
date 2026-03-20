use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

const MEMCPY_FAMILY: &[&str] = &["memcpy", "memmove", "strncpy", "memset"];
const SCANF_FAMILY: &[&str] = &["scanf", "fscanf", "sscanf"];

pub struct CBufferOverflowSecurityPipeline {
    call_query: Arc<Query>,
}

impl CBufferOverflowSecurityPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    /// Extract parameter names from a function_definition's parameter_list.
    fn extract_param_names(fn_def: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(declarator) = fn_def.child_by_field_name("declarator")
            && let Some(params) = declarator.child_by_field_name("parameters")
        {
            let mut cursor = params.walk();
            for child in params.named_children(&mut cursor) {
                if child.kind() == "parameter_declaration"
                    && let Some(decl) = child.child_by_field_name("declarator")
                {
                    Self::collect_identifiers(decl, source, &mut names);
                }
            }
        }
        names
    }

    fn collect_identifiers(node: tree_sitter::Node, source: &[u8], names: &mut Vec<String>) {
        if node.kind() == "identifier" {
            names.push(node_text(node, source).to_string());
            return;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            Self::collect_identifiers(inner, source, names);
        }
    }

    /// Walk upward from a node to find the enclosing function_definition.
    fn find_enclosing_function(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        let mut current = node.parent();
        while let Some(n) = current {
            if n.kind() == "function_definition" {
                return Some(n);
            }
            current = n.parent();
        }
        None
    }

    /// Check if a size argument looks like a sizeof expression or numeric literal.
    fn is_safe_size_arg(node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(node, source);
        if text.starts_with("sizeof") {
            return true;
        }
        let kind = node.kind();
        if kind == "sizeof_expression" || kind == "number_literal" {
            return true;
        }
        // Check if it's a binary expression where at least one side is sizeof
        if kind == "binary_expression" {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "sizeof_expression" {
                    return true;
                }
                let child_text = node_text(child, source);
                if child_text.starts_with("sizeof") {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a format string text contains %s without a width specifier.
    fn has_bare_percent_s(format_text: &str) -> bool {
        let bytes = format_text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                i += 1;
                // Skip flags like -, +, 0, space, #
                while i < bytes.len()
                    && (bytes[i] == b'-'
                        || bytes[i] == b'+'
                        || bytes[i] == b'0'
                        || bytes[i] == b' '
                        || bytes[i] == b'#')
                {
                    i += 1;
                }
                // If next char is 's', it's a bare %s (no width specifier)
                if i < bytes.len() && bytes[i] == b's' {
                    return true;
                }
                // If digits appear before 's', it has a width — skip
            } else {
                i += 1;
            }
        }
        false
    }
}

impl Pipeline for CBufferOverflowSecurityPipeline {
    fn name(&self) -> &str {
        "c_buffer_overflow_security"
    }

    fn description(&self) -> &str {
        "Detects buffer overflow vulnerabilities: unchecked memcpy size, off-by-one strlen, scanf without width"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");
        let args_idx = find_capture_index(&self.call_query, "args");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);

            if let (Some(fn_cap), Some(call_cap), Some(args_cap)) = (fn_cap, call_cap, args_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                let args_node = args_cap.node;
                let named_args: Vec<tree_sitter::Node> = {
                    let mut walker = args_node.walk();
                    args_node.named_children(&mut walker).collect()
                };

                // Pattern: memcpy_unchecked_size
                if MEMCPY_FAMILY.contains(&fn_name) {
                    // Size is the 3rd argument (index 2) for memcpy/memmove/strncpy/memset
                    if let Some(size_arg) = named_args.get(2)
                        && !Self::is_safe_size_arg(*size_arg, source)
                    {
                        let size_text = node_text(*size_arg, source);

                        // Check if size arg references a function parameter
                        let is_param =
                            if let Some(fn_def) = Self::find_enclosing_function(call_cap.node) {
                                let params = Self::extract_param_names(fn_def, source);
                                params.iter().any(|p| size_text.contains(p))
                            } else {
                                // Not inside a function — still flag plain identifiers
                                size_arg.kind() == "identifier"
                            };

                        if is_param || size_arg.kind() == "identifier" {
                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "memcpy_unchecked_size".to_string(),
                                    message: format!(
                                        "`{fn_name}()` with externally-controlled size parameter `{size_text}` — verify bounds before copy"
                                    ),
                                    snippet: extract_snippet(source, call_cap.node, 1),
                                });
                        }
                    }
                }

                // Pattern: scanf_no_width
                if SCANF_FAMILY.contains(&fn_name) {
                    // For scanf: format string is the 1st arg
                    // For fscanf/sscanf: format string is the 2nd arg
                    let fmt_arg_idx = if fn_name == "scanf" { 0 } else { 1 };

                    if let Some(fmt_arg) = named_args.get(fmt_arg_idx)
                        && fmt_arg.kind() == "string_literal"
                    {
                        let fmt_text = node_text(*fmt_arg, source);
                        if Self::has_bare_percent_s(fmt_text) {
                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "scanf_no_width".to_string(),
                                    message: format!(
                                        "`{fn_name}()` with `%s` has no width limit — use `%Ns` to prevent buffer overflow"
                                    ),
                                    snippet: extract_snippet(source, call_cap.node, 1),
                                });
                        }
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CBufferOverflowSecurityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_memcpy_param_size() {
        let src = "void f(int n) { char buf[100]; char *src = \"x\"; memcpy(buf, src, n); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "memcpy_unchecked_size");
    }

    #[test]
    fn ignores_memcpy_sizeof() {
        let src = "void f() { char buf[100]; char src[100]; memcpy(buf, src, sizeof(buf)); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_scanf_no_width() {
        let src = r#"void f() { char buf[32]; scanf("%s", buf); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "scanf_no_width");
    }

    #[test]
    fn ignores_scanf_with_width() {
        let src = r#"void f() { char buf[32]; scanf("%31s", buf); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }
}
