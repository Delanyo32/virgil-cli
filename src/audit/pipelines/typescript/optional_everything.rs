use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_interface_declaration_query, extract_snippet, find_capture_index, is_dts_file,
    is_ts_suppressed, node_text,
};

fn compile_type_alias_object_query(language: Language) -> anyhow::Result<Arc<Query>> {
    let query_str = r#"
(type_alias_declaration
  name: (type_identifier) @name
  value: (object_type) @body) @decl
"#;
    let q = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile type_alias_object query")?;
    Ok(Arc::new(q))
}

use anyhow::Context;

pub struct OptionalEverythingPipeline {
    query: Arc<Query>,
    name_idx: usize,
    body_idx: usize,
    type_alias_query: Arc<Query>,
    ta_name_idx: usize,
    ta_body_idx: usize,
}

impl OptionalEverythingPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_interface_declaration_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let body_idx = find_capture_index(&query, "body");
        let type_alias_query = compile_type_alias_object_query(language)?;
        let ta_name_idx = find_capture_index(&type_alias_query, "name");
        let ta_body_idx = find_capture_index(&type_alias_query, "body");
        Ok(Self {
            query,
            name_idx,
            body_idx,
            type_alias_query,
            ta_name_idx,
            ta_body_idx,
        })
    }
}

fn count_optional_props(body_node: tree_sitter::Node, source: &[u8]) -> (u32, u32) {
    let mut total_props = 0u32;
    let mut optional_props = 0u32;
    let mut body_cursor = body_node.walk();
    for child in body_node.named_children(&mut body_cursor) {
        if child.kind() == "property_signature" {
            total_props += 1;
            let mut child_walk = child.walk();
            for token in child.children(&mut child_walk) {
                if !token.is_named() && node_text(token, source) == "?" {
                    optional_props += 1;
                    break;
                }
            }
        }
    }
    (total_props, optional_props)
}

fn is_options_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with("options")
        || n.ends_with("config")
        || n.ends_with("props")
        || n.ends_with("settings")
        || n.ends_with("params")
        || n.ends_with("args")
}

impl Pipeline for OptionalEverythingPipeline {
    fn name(&self) -> &str {
        "optional_everything"
    }

    fn description(&self) -> &str {
        "Detects interfaces where most properties are optional, suggesting a design smell"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_dts_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // -- Interface declarations --
        {
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

                let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);

                if is_ts_suppressed(source, decl_node) {
                    continue;
                }

                let (total_props, optional_props) = count_optional_props(body_node, source);

                if total_props >= 5 && optional_props as f64 / total_props as f64 > 0.6 {
                    let start = decl_node.start_position();
                    let severity = if is_options_name(iface_name) {
                        "info"
                    } else {
                        "info"
                    };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "optional_overload".to_string(),
                        message: format!(
                            "Interface `{iface_name}` has {optional_props}/{total_props} optional properties — consider splitting into required and optional parts"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        // -- Type alias declarations with object types --
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.type_alias_query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                let type_name = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == self.ta_name_idx)
                    .map(|c| node_text(c.node, source))
                    .unwrap_or("<anonymous>");

                let body_node = match m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == self.ta_body_idx)
                {
                    Some(c) => c.node,
                    None => continue,
                };

                let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);

                if is_ts_suppressed(source, decl_node) {
                    continue;
                }

                let (total_props, optional_props) = count_optional_props(body_node, source);

                if total_props >= 5 && optional_props as f64 / total_props as f64 > 0.6 {
                    let start = decl_node.start_position();
                    let severity = if is_options_name(type_name) { "info" } else { "info" };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "optional_overload".to_string(),
                        message: format!(
                            "Type `{type_name}` has {optional_props}/{total_props} optional properties — consider splitting into required and optional parts"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = OptionalEverythingPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = OptionalEverythingPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
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
    fn detects_optional_type_alias() {
        let src = "type Config = { a?: string; b?: number; c?: boolean; d?: string; e?: number; f: string; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn options_naming_does_not_produce_warning() {
        let src = "interface RequestOptions { a?: string; b?: number; c?: boolean; d?: string; e?: number; f: string; }";
        let findings = parse_and_check(src);
        assert!(findings.iter().all(|f| f.severity != "warning"));
    }

    #[test]
    fn skips_dts_file() {
        let src = "interface Big { a?: string; b?: number; c?: boolean; d?: string; e?: number; f: string; }";
        let findings = parse_and_check_path(src, "types.d.ts");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_optional_interface() {
        let src = "// virgil-ignore\ninterface Big { a?: string; b?: number; c?: boolean; d?: string; e?: number; f: string; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        OptionalEverythingPipeline::new(Language::Tsx).unwrap();
    }
}
