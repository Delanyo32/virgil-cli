use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{compile_call_expression_query, extract_snippet, find_capture_index};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

struct FunctionInfo {
    severity: &'static str,
    safe_alternative: &'static str,
}

fn function_info(name: &str) -> Option<FunctionInfo> {
    match name {
        "gets" => Some(FunctionInfo {
            severity: "error",
            safe_alternative: "use `fgets()`",
        }),
        "strcpy" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `strncpy()` or `strlcpy()`",
        }),
        "strcat" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `strncat()` or `strlcat()`",
        }),
        "sprintf" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `snprintf()`",
        }),
        "vsprintf" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `vsnprintf()`",
        }),
        "scanf" | "sscanf" | "vscanf" | "vsscanf" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use width-limited format specifiers or `fgets()` + `sscanf()`",
        }),
        "wcscpy" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `wcsncpy()`",
        }),
        "wcscat" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use `wcsncat()`",
        }),
        "swprintf" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use bounded `swprintf()` with size parameter",
        }),
        "stpcpy" | "stpncpy" => Some(FunctionInfo {
            severity: "warning",
            safe_alternative: "use bounded copy with explicit size checks",
        }),
        _ => None,
    }
}

pub struct BufferOverflowsPipeline {
    call_query: Arc<Query>,
}

impl BufferOverflowsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

impl GraphPipeline for BufferOverflowsPipeline {
    fn name(&self) -> &str {
        "buffer_overflows"
    }

    fn description(&self) -> &str {
        "Detects usage of unsafe string functions (strcpy, sprintf, gets, etc.) that can cause buffer overflows"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(call_cap)) = (fn_cap, call_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                let info = match function_info(fn_name) {
                    Some(i) => i,
                    None => continue,
                };

                if is_nolint_suppressed(source, call_cap.node, self.name()) {
                    continue;
                }

                let start = call_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: info.severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unsafe_string_function".to_string(),
                    message: format!(
                        "`{fn_name}()` is unsafe — {}",
                        info.safe_alternative
                    ),
                    snippet: extract_snippet(source, call_cap.node, 1),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = BufferOverflowsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.c",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_strcpy() {
        let src = "void f() { strcpy(dest, src); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_string_function");
        assert!(findings[0].message.contains("strcpy"));
        assert_eq!(findings[0].severity, "warning");
        assert!(findings[0].message.contains("strncpy"));
    }

    #[test]
    fn detects_sprintf() {
        let src = "void f() { sprintf(buf, \"%s\", name); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("sprintf"));
        assert!(findings[0].message.contains("snprintf"));
    }

    #[test]
    fn detects_gets() {
        let src = "void f() { gets(buf); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("gets"));
        assert_eq!(findings[0].severity, "error");
        assert!(findings[0].message.contains("fgets"));
    }

    #[test]
    fn skips_safe_alternatives() {
        let src = "void f() { strncpy(dest, src, sizeof(dest)); snprintf(buf, sizeof(buf), \"%s\", name); memcpy(dest, src, n); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_expanded_functions() {
        let src = r#"
void f() {
    wcscpy(wdest, wsrc);
    sscanf(buf, "%d", &x);
    stpcpy(dest, src);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "void f() { strcpy(dest, src); } // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_targeted_suppresses_only_named() {
        let src = "void f() { strcpy(dest, src); } // NOLINT(buffer_overflows)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_targeted_other_pipeline_does_not_suppress() {
        let src = "void f() { strcpy(dest, src); } // NOLINT(other_pipeline)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn severity_graduation_gets_is_error() {
        let src = "void f() { gets(buf); }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn severity_graduation_strcpy_is_warning() {
        let src = "void f() { strcpy(dest, src); }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn multiple_unsafe_in_one_function() {
        let src = "void f() { strcpy(dest, src); sprintf(buf, \"%s\", name); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }
}
