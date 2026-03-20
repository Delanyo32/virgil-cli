use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

pub struct CppInjectionPipeline {
    call_query: Arc<Query>,
}

impl CppInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

fn matches_function(fn_text: &str, target: &str) -> bool {
    fn_text == target || fn_text.ends_with(&format!("::{target}"))
}

impl Pipeline for CppInjectionPipeline {
    fn name(&self) -> &str {
        "cpp_injection"
    }

    fn description(&self) -> &str {
        "Detects injection risks: system()/popen() with string concatenation, printf with variable format"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(args_cap), Some(call_cap)) = (fn_cap, args_cap, call_cap) {
                let fn_text = node_text(fn_cap.node, source);
                let args_text = node_text(args_cap.node, source);

                // Pattern: system() with dynamic argument
                if matches_function(fn_text, "system") || matches_function(fn_text, "std::system") {
                    // Check if argument contains string concatenation or dynamic content
                    let has_dynamic = args_text.contains(".c_str()")
                        || args_text.contains('+')
                        || !self.first_arg_is_string_literal(args_cap.node, source);

                    if has_dynamic {
                        let start = call_cap.node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "system_string_concat".to_string(),
                            message: "`system()` with dynamic argument — risk of command injection"
                                .to_string(),
                            snippet: extract_snippet(source, call_cap.node, 1),
                        });
                        continue;
                    }
                }

                // Pattern: popen() with non-literal command
                if (matches_function(fn_text, "popen") || matches_function(fn_text, "_popen"))
                    && !self.first_arg_is_string_literal(args_cap.node, source)
                {
                    let start = call_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "popen_non_literal".to_string(),
                        message: "`popen()` with non-literal command — risk of command injection"
                            .to_string(),
                        snippet: extract_snippet(source, call_cap.node, 1),
                    });
                    continue;
                }

                // Pattern: printf family with variable format string
                let printf_fns = ["printf", "fprintf", "sprintf", "snprintf"];
                for &target in &printf_fns {
                    if matches_function(fn_text, target) {
                        let format_arg_is_literal = if target == "printf" {
                            self.first_arg_is_string_literal(args_cap.node, source)
                        } else {
                            // fprintf/sprintf/snprintf: second arg is the format string
                            self.second_arg_is_string_literal(args_cap.node, source)
                        };

                        if !format_arg_is_literal {
                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "printf_variable_format".to_string(),
                                message: format!(
                                    "`{target}()` with variable format string — risk of format string injection"
                                ),
                                snippet: extract_snippet(source, call_cap.node, 1),
                            });
                        }
                        break;
                    }
                }
            }
        }

        findings
    }
}

impl CppInjectionPipeline {
    fn first_arg_is_string_literal(&self, args_node: tree_sitter::Node, source: &[u8]) -> bool {
        self.nth_named_arg_is_string_literal(args_node, source, 0)
    }

    fn second_arg_is_string_literal(&self, args_node: tree_sitter::Node, source: &[u8]) -> bool {
        self.nth_named_arg_is_string_literal(args_node, source, 1)
    }

    fn nth_named_arg_is_string_literal(
        &self,
        args_node: tree_sitter::Node,
        _source: &[u8],
        n: usize,
    ) -> bool {
        let mut named_idx = 0;
        let mut cursor = args_node.walk();
        for child in args_node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if named_idx == n {
                return child.kind() == "string_literal";
            }
            named_idx += 1;
        }
        false
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
        let pipeline = CppInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_system_concat() {
        let src = "void f(std::string cmd) { system(cmd.c_str()); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "system_string_concat");
    }

    #[test]
    fn ignores_system_literal() {
        let src = r#"void f() { system("ls"); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_printf_variable() {
        let src = "void f(const char *fmt) { printf(fmt); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "printf_variable_format");
    }

    #[test]
    fn ignores_printf_literal() {
        let src = r#"void f() { printf("hello %d", 42); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_popen_variable() {
        let src = "void f(const char *cmd) { popen(cmd, \"r\"); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "popen_non_literal");
    }

    #[test]
    fn metadata_correct() {
        let src = "void f(const char *fmt) { printf(fmt); }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].pipeline, "cpp_injection");
    }
}
