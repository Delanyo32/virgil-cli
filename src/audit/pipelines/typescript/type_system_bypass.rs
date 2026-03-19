use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_as_expression_query, compile_non_null_assertion_query, extract_snippet,
    find_capture_index, node_text,
};

const UNTRUSTED_SOURCES: &[&str] = &[
    "JSON.parse",
    ".json()",
    "req.body",
    "req.params",
    "req.query",
    "request.body",
    "request.params",
    "request.query",
];

pub struct TypeSystemBypassPipeline {
    as_expr_query: Arc<Query>,
    non_null_query: Arc<Query>,
}

impl TypeSystemBypassPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            as_expr_query: compile_as_expression_query(language)?,
            non_null_query: compile_non_null_assertion_query(language)?,
        })
    }
}

impl Pipeline for TypeSystemBypassPipeline {
    fn name(&self) -> &str {
        "type_system_bypass"
    }

    fn description(&self) -> &str {
        "Detects unsafe type system bypasses: `as` cast on untrusted data, `!` on untrusted data, double cast"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // as expression on untrusted data + double cast detection
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.as_expr_query, tree.root_node(), source);
            let as_idx = find_capture_index(&self.as_expr_query, "as_expr");

            while let Some(m) = matches.next() {
                let as_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == as_idx)
                    .map(|c| c.node);

                if let Some(as_expr) = as_node {
                    let expr_text = node_text(as_expr, source);

                    // Check for double cast: as unknown as T (nested as_expression)
                    let inner = as_expr
                        .child_by_field_name("expression")
                        .or_else(|| as_expr.named_child(0));
                    if let Some(inner_node) = inner {
                        if inner_node.kind() == "as_expression" {
                            let start = as_expr.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "double_cast_escape".to_string(),
                                message:
                                    "Double type assertion (`as X as Y`) — bypasses type safety"
                                        .to_string(),
                                snippet: extract_snippet(source, as_expr, 1),
                            });
                            continue;
                        }

                        // Check if inner expression references untrusted source
                        let inner_text = node_text(inner_node, source);
                        let is_untrusted = UNTRUSTED_SOURCES.iter().any(|s| inner_text.contains(s));

                        if is_untrusted {
                            let start = as_expr.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unsafe_cast_untrusted_data".to_string(),
                                message: format!(
                                    "Type assertion on untrusted data `{}` — validate at runtime",
                                    truncate(expr_text, 60)
                                ),
                                snippet: extract_snippet(source, as_expr, 1),
                            });
                        }
                    }
                }
            }
        }

        // Non-null assertion on untrusted data
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.non_null_query, tree.root_node(), source);
            let nn_idx = find_capture_index(&self.non_null_query, "non_null");

            while let Some(m) = matches.next() {
                let nn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == nn_idx)
                    .map(|c| c.node);

                if let Some(nn) = nn_node {
                    let inner_text = node_text(nn, source);

                    // Check if the inner expression is an untrusted source
                    let is_untrusted = inner_text.contains("req.headers")
                        || inner_text.contains("request.headers")
                        || inner_text.contains(".get(")
                        || inner_text.contains("Map.get")
                        || UNTRUSTED_SOURCES.iter().any(|s| inner_text.contains(s));

                    if is_untrusted {
                        let start = nn.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "non_null_untrusted_data".to_string(),
                            message: "Non-null assertion `!` on potentially null/undefined value from untrusted source".to_string(),
                            snippet: extract_snippet(source, nn, 1),
                        });
                    }
                }
            }
        }

        findings
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::TypeScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypeSystemBypassPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_as_cast_on_json_parse() {
        let src = "const user = JSON.parse(data) as User;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_cast_untrusted_data");
    }

    #[test]
    fn detects_as_cast_on_req_body() {
        let src = "const body = req.body as CreateUserDTO;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_cast_untrusted_data");
    }

    #[test]
    fn detects_double_cast() {
        let src = "const x = value as unknown as SecretType;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "double_cast_escape");
    }

    #[test]
    fn detects_non_null_on_headers() {
        let src = "const auth = req.headers.get('authorization')!;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "non_null_untrusted_data");
    }

    #[test]
    fn ignores_safe_as_cast() {
        let src = "const x = someLocalVar as string;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        TypeSystemBypassPipeline::new(Language::Tsx).unwrap();
    }
}
