use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_expression_statement_call_query, extract_snippet, find_capture_index,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

const DANGEROUS_FUNCTIONS: &[&str] = &[
    "fwrite", "fread", "fclose", "fopen", "fgets", "fputs", "strncpy", "snprintf", "memcpy",
    "memmove", "read", "write", "close", "open", "pthread_create", "recv", "send", "connect",
    "bind", "listen", "accept", "socket",
];

fn severity_for(fn_name: &str) -> &'static str {
    match fn_name {
        "fopen" | "open" | "socket" => "error",
        "fwrite" | "fread" | "read" | "write" | "fgets" | "fputs" | "recv" | "send"
        | "connect" | "bind" | "listen" | "accept" | "pthread_create" | "snprintf" => "warning",
        "memcpy" | "memmove" | "strncpy" | "close" | "fclose" => "info",
        _ => "warning",
    }
}

pub struct IgnoredReturnValuesPipeline {
    expr_call_query: Arc<Query>,
}

impl IgnoredReturnValuesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            expr_call_query: compile_expression_statement_call_query()?,
        })
    }
}

impl GraphPipeline for IgnoredReturnValuesPipeline {
    fn name(&self) -> &str {
        "ignored_return_values"
    }

    fn description(&self) -> &str {
        "Detects discarded return values from functions whose return value indicates success/failure"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.expr_call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.expr_call_query, "fn_name");
        let call_idx = find_capture_index(&self.expr_call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(call_cap)) = (fn_cap, call_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                if !DANGEROUS_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                if is_nolint_suppressed(source, call_cap.node, self.name()) {
                    continue;
                }

                let start = call_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity_for(fn_name).to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "ignored_return_value".to_string(),
                    message: format!(
                        "return value of `{fn_name}()` is discarded — check for errors"
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
        let pipeline = IgnoredReturnValuesPipeline::new().unwrap();
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
    fn detects_ignored_fwrite() {
        let src = "void f() { fwrite(buf, 1, n, fp); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ignored_return_value");
        assert!(findings[0].message.contains("fwrite"));
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn skips_assigned_return() {
        let src = "void f() { size_t n = fwrite(buf, 1, sz, fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_ignored_fclose() {
        let src = "void f() { fclose(fp); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fclose"));
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn void_cast_suppresses() {
        // (void) cast wraps the call_expression in a cast_expression,
        // so it is no longer a direct child of expression_statement
        let src = "void f() { (void)fclose(fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn fopen_is_error_severity() {
        let src = "void f() { fopen(\"file.txt\", \"r\"); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn memcpy_is_info_severity() {
        let src = "void f() { memcpy(dest, src, n); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn detects_expanded_functions() {
        let src = r#"
void f() {
    recv(sock, buf, len, 0);
    pthread_create(&t, 0, func, arg);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn nolint_suppresses() {
        let src = "void f() { fwrite(buf, 1, n, fp); } // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
