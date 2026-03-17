use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_interface_declaration_query, extract_snippet, find_capture_index, node_text,
};

pub struct OptionalEverythingPipeline {
    query: Arc<Query>,
    name_idx: usize,
    body_idx: usize,
}

impl OptionalEverythingPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_interface_declaration_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let body_idx = find_capture_index(&query, "body");
        Ok(Self {
            query,
            name_idx,
            body_idx,
        })
    }
}

impl Pipeline for OptionalEverythingPipeline {
    fn name(&self) -> &str {
        "optional_everything"
    }

    fn description(&self) -> &str {
        "Detects interfaces where most properties are optional, suggesting a design smell"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let iface_name = m
                .captures
                .iter()
                .find(|c| c.index as usize == self.name_idx)
                .map(|c| node_text(c.node, source))
                .unwrap_or("<anonymous>");

            let body_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.body_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            let mut total_props = 0u32;
            let mut optional_props = 0u32;
            let mut body_cursor = body_node.walk();

            for child in body_node.named_children(&mut body_cursor) {
                if child.kind() == "property_signature" {
                    total_props += 1;
                    // Check for `?` token among anonymous children
                    let mut child_walk = child.walk();
                    for token in child.children(&mut child_walk) {
                        if !token.is_named() && node_text(token, source) == "?" {
                            optional_props += 1;
                            break;
                        }
                    }
                }
            }

            if total_props >= 5 && optional_props as f64 / total_props as f64 > 0.6 {
                let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "optional_overload".to_string(),
                    message: format!(
                        "Interface `{iface_name}` has {optional_props}/{total_props} optional properties — consider splitting into required and optional parts"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = OptionalEverythingPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_mostly_optional_interface() {
        let src = r#"
interface Config {
    a?: string;
    b?: number;
    c?: boolean;
    d?: string;
    e?: number;
    f: string;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "optional_overload");
    }

    #[test]
    fn skips_small_interface() {
        let src = r#"
interface Small {
    a?: string;
    b?: number;
    c?: boolean;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_mostly_required_interface() {
        let src = r#"
interface Solid {
    a: string;
    b: number;
    c: boolean;
    d: string;
    e: number;
    f?: string;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn boundary_exactly_60_percent() {
        // 3 optional out of 5 = exactly 60%, which is NOT > 60%
        let src = r#"
interface Boundary {
    a?: string;
    b?: number;
    c?: boolean;
    d: string;
    e: number;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        OptionalEverythingPipeline::new(Language::Tsx).unwrap();
    }
}
