use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

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
    "PathBuf",
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

fn param_is_in_trait_or_trait_impl(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "impl_item" {
            // It's a trait impl if the impl_item has a "trait" field
            return p.child_by_field_name("trait").is_some();
        }
        if p.kind() == "trait_item" {
            // Parameters inside a trait definition cannot be changed either
            return true;
        }
        current = p.parent();
    }
    false
}

impl GraphPipeline for MissingTraitAbstractionPipeline {
    fn name(&self) -> &str {
        "missing_trait_abstraction"
    }

    fn description(&self) -> &str {
        "Detects function parameters using concrete infrastructure types instead of trait abstractions (e.g. File instead of impl Read)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        if is_test_file(file_path) {
            return vec![];
        }

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
                // Skip params in trait definitions or trait impl methods — cannot change the signature
                if param_is_in_trait_or_trait_impl(param_cap.node) {
                    continue;
                }

                // Skip exempted functions: main, new, open*
                let skip = {
                    let mut node = param_cap.node;
                    let mut should_skip = false;
                    while let Some(parent) = node.parent() {
                        if parent.kind() == "function_item" {
                            if let Some(name_node) = parent.child_by_field_name("name") {
                                let fn_name = name_node.utf8_text(source).unwrap_or("");
                                if fn_name == "main"
                                    || fn_name == "new"
                                    || fn_name.starts_with("open")
                                {
                                    should_skip = true;
                                }
                            }
                            break;
                        }
                        node = parent;
                    }
                    should_skip
                };
                if skip {
                    continue;
                }

                let type_text = type_cap.node.utf8_text(source).unwrap_or("");
                let leaf = self.extract_leaf_type(type_text);

                if CONCRETE_INFRA_TYPES.contains(&leaf) {
                    // Determine severity: exported fn → "warning", private fn → "info"
                    let severity = {
                        let mut node = param_cap.node;
                        let mut is_exported = false;
                        while let Some(parent) = node.parent() {
                            if parent.kind() == "function_item" {
                                // Check for visibility_modifier child
                                for i in 0..parent.named_child_count() {
                                    if let Some(child) = parent.named_child(i) {
                                        if child.kind() == "visibility_modifier" {
                                            is_exported = true;
                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                            node = parent;
                        }
                        if is_exported { "warning" } else { "info" }
                    };

                    let start = param_cap.node.start_position();
                    let snippet = param_cap.node.utf8_text(source).unwrap_or("").to_string();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
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
        parse_and_check_path(source, "test.rs")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTraitAbstractionPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn detects_pathbuf_param() {
        let src = r#"fn process(path: PathBuf) {}"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty(), "PathBuf param should be flagged");
    }

    #[test]
    fn trait_impl_method_not_flagged() {
        let src = r#"
trait Handler { fn handle(&self, f: std::fs::File); }
impl Handler for MyType { fn handle(&self, f: std::fs::File) {} }
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "trait impl params cannot be changed");
    }

    #[test]
    fn test_file_excluded() {
        let src = r#"fn process(f: std::fs::File) {}"#;
        let findings = parse_and_check_path(src, "tests/helpers.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn pub_fn_is_warning() {
        let src = r#"pub fn process(f: File) {}"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn private_fn_is_info() {
        let src = r#"fn process(f: File) {}"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "info");
    }
}
