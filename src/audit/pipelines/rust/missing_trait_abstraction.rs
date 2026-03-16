use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::rust_primitives as primitives;

const CONCRETE_INFRA_TYPES: &[&str] = &[
    "File",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "BufReader",
    "BufWriter",
    "Stdin",
    "Stdout",
    "Stderr",
];

pub struct MissingTraitAbstractionPipeline {
    param_query: Arc<Query>,
}

impl MissingTraitAbstractionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: primitives::compile_parameter_query()?,
        })
    }

    fn extract_leaf_type<'a>(&self, type_text: &'a str) -> &'a str {
        let stripped = type_text
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim();
        // Get the last path segment (e.g. "std::fs::File" -> "File")
        stripped.rsplit("::").next().unwrap_or(stripped)
    }
}

impl Pipeline for MissingTraitAbstractionPipeline {
    fn name(&self) -> &str {
        "missing_trait_abstraction"
    }

    fn description(&self) -> &str {
        "Detects function parameters using concrete infrastructure types instead of trait abstractions (e.g. File instead of impl Read)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.param_query, tree.root_node(), source);

        let type_idx = self
            .param_query
            .capture_names()
            .iter()
            .position(|n| *n == "param_type")
            .unwrap();
        let param_idx = self
            .param_query
            .capture_names()
            .iter()
            .position(|n| *n == "param")
            .unwrap();

        while let Some(m) = matches.next() {
            let type_node = m.captures.iter().find(|c| c.index as usize == type_idx);
            let param_node = m.captures.iter().find(|c| c.index as usize == param_idx);

            if let (Some(type_cap), Some(param_cap)) = (type_node, param_node) {
                let type_text = type_cap.node.utf8_text(source).unwrap_or("");
                let leaf = self.extract_leaf_type(type_text);

                if CONCRETE_INFRA_TYPES.contains(&leaf) {
                    let start = param_cap.node.start_position();
                    let snippet = param_cap.node.utf8_text(source).unwrap_or("").to_string();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "concrete_infra_type".to_string(),
                        message: format!(
                            "parameter uses concrete type `{leaf}` — consider using a trait like `impl Read`/`impl Write` for better testability"
                        ),
                        snippet,
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTraitAbstractionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_file_param() {
        let src = r#"
fn process(file: File) {}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "concrete_infra_type");
        assert!(findings[0].message.contains("File"));
    }

    #[test]
    fn detects_ref_file_param() {
        let src = r#"
fn process(file: &File) {}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_trait_impl_param() {
        let src = r#"
fn process(reader: impl Read) {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_infra_type() {
        let src = r#"
fn process(config: Config) {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_tcp_stream() {
        let src = r#"
fn handle(stream: TcpStream) {}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
