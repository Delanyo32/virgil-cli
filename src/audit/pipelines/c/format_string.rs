use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_expression_query, extract_snippet, find_capture_index};

const PRINTF_FAMILY_NO_SKIP: &[&str] = &["printf", "syslog", "dprintf"];
const PRINTF_FAMILY_SKIP_FIRST: &[&str] = &["fprintf", "sprintf", "snprintf"];

pub struct FormatStringPipeline {
    call_query: Arc<Query>,
}

impl FormatStringPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    fn is_safe_format_kind(kind: &str) -> bool {
        kind == "string_literal" || kind == "concatenated_string"
    }
}

impl Pipeline for FormatStringPipeline {
    fn name(&self) -> &str {
        "format_string"
    }

    fn description(&self) -> &str {
        "Detects format string vulnerabilities: printf-family calls where format argument is a variable, not a string literal"
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

                // Collect named children of the argument_list (skip parentheses)
                let args_node = args_cap.node;
                let named_args: Vec<tree_sitter::Node> = {
                    let mut walker = args_node.walk();
                    args_node.named_children(&mut walker).collect()
                };

                let format_arg = if PRINTF_FAMILY_NO_SKIP.contains(&fn_name) {
                    // Format string is the first argument
                    named_args.first().copied()
                } else if PRINTF_FAMILY_SKIP_FIRST.contains(&fn_name) {
                    // Format string is the second argument (skip file/buffer/size)
                    named_args.get(1).copied()
                } else {
                    continue;
                };

                if let Some(fmt_node) = format_arg
                    && !Self::is_safe_format_kind(fmt_node.kind()) {
                        let start = call_cap.node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "format_string_variable".to_string(),
                            message: format!(
                                "`{fn_name}()` called with variable format string — risk of format string injection"
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
        let pipeline = FormatStringPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_printf_variable_format() {
        let src = "void f(char *s) { printf(s); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "format_string_variable");
        assert!(findings[0].message.contains("printf"));
    }

    #[test]
    fn ignores_printf_literal_format() {
        let src = r#"void f() { printf("hello %s", name); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_sprintf_variable_format() {
        let src = "void f(char *buf, char *fmt) { sprintf(buf, fmt); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "format_string_variable");
        assert!(findings[0].message.contains("sprintf"));
    }
}
