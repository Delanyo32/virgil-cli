use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_generic_type_query, extract_snippet, find_capture_index, is_test_file, is_ts_suppressed,
    node_text,
};

pub struct RecordStringAnyPipeline {
    query: Arc<Query>,
    name_idx: usize,
    args_idx: usize,
    index_sig_query: Arc<Query>,
}

impl RecordStringAnyPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_generic_type_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let args_idx = find_capture_index(&query, "args");
        let index_sig_query = compile_index_signature_query(language)?;
        Ok(Self {
            query,
            name_idx,
            args_idx,
            index_sig_query,
        })
    }
}

fn compile_index_signature_query(language: Language) -> Result<Arc<Query>> {
    // Match index signatures like { [key: string]: any }
    let query_str = r#"
(index_signature) @idx_sig
"#;
    let q = Query::new(&language.tree_sitter_language(), query_str)
        .with_context(|| "failed to compile index_signature query")?;
    Ok(Arc::new(q))
}

impl Pipeline for RecordStringAnyPipeline {
    fn name(&self) -> &str {
        "record_string_any"
    }

    fn description(&self) -> &str {
        "Detects `Record<string, any>` which is a type-unsafe catch-all"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let in_test = is_test_file(file_path);
        let mut findings = Vec::new();

        // -- Record<string, any> --
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                let type_name = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == self.name_idx)
                    .map(|c| node_text(c.node, source))
                    .unwrap_or("");

                if type_name != "Record" {
                    continue;
                }

                let args_node = match m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == self.args_idx)
                {
                    Some(c) => c.node,
                    None => continue,
                };

                let mut has_any = false;
                let mut args_cursor = args_node.walk();
                for child in args_node.named_children(&mut args_cursor) {
                    if child.kind() == "predefined_type" && node_text(child, source) == "any" {
                        has_any = true;
                        break;
                    }
                }

                if has_any {
                    let generic_node = m.captures.first().map(|c| c.node).unwrap_or(args_node);
                    if is_ts_suppressed(source, generic_node) {
                        continue;
                    }
                    let severity = if in_test { "info" } else { "warning" };
                    let start = generic_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "record_any".to_string(),
                        message: "`Record<string, any>` is a type-unsafe catch-all — define a specific value type or use `unknown`".to_string(),
                        snippet: extract_snippet(source, generic_node, 1),
                    });
                }
            }
        }

        // -- Index signatures: { [key: string]: any } --
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.index_sig_query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                let idx_sig_node = match m.captures.first() {
                    Some(c) => c.node,
                    None => continue,
                };

                // Check if the value type is `any`
                // Index signature structure: [name: type]: value_type
                // The last named child should be the value type
                let sig_text = node_text(idx_sig_node, source);
                // Quick check: must contain "any"
                if !sig_text.contains("any") {
                    continue;
                }

                // More precise: walk named children to find predefined_type = "any"
                // as the value type (not the key type)
                let mut has_any_value = false;
                let mut sig_cursor = idx_sig_node.walk();
                let named_children: Vec<_> = idx_sig_node
                    .named_children(&mut sig_cursor)
                    .collect();
                // The value type is typically the last named child
                if let Some(last) = named_children.last() {
                    if last.kind() == "predefined_type" && node_text(*last, source) == "any" {
                        has_any_value = true;
                    }
                    // Also handle type_annotation wrapper
                    if last.kind() == "type_annotation" {
                        let mut ta_cursor = last.walk();
                        for child in last.named_children(&mut ta_cursor) {
                            if child.kind() == "predefined_type" && node_text(child, source) == "any" {
                                has_any_value = true;
                            }
                        }
                    }
                }

                if !has_any_value {
                    // Fallback: check if any named child except the first is predefined_type any
                    let mut sig_cursor2 = idx_sig_node.walk();
                    let children: Vec<_> = idx_sig_node.named_children(&mut sig_cursor2).collect();
                    for child in children.iter().skip(1) {
                        if child.kind() == "predefined_type" && node_text(*child, source) == "any" {
                            has_any_value = true;
                            break;
                        }
                    }
                }

                if has_any_value {
                    if is_ts_suppressed(source, idx_sig_node) {
                        continue;
                    }
                    let severity = if in_test { "info" } else { "warning" };
                    let start = idx_sig_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "index_signature_any".to_string(),
                        message: "Index signature with `any` value type is type-unsafe — use `unknown` or a specific type".to_string(),
                        snippet: extract_snippet(source, idx_sig_node, 1),
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
        let pipeline = RecordStringAnyPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RecordStringAnyPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
    }

    #[test]
    fn detects_record_string_any() {
        let findings = parse_and_check("let x: Record<string, any> = {};");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "record_any");
    }

    #[test]
    fn skips_record_string_number() {
        let findings = parse_and_check("let x: Record<string, number> = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_record_generic() {
        let findings = parse_and_check("let x: Map<string, any>;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_record_string_unknown() {
        let findings = parse_and_check("let x: Record<string, unknown> = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_index_signature_any() {
        let findings = parse_and_check("let x: { [key: string]: any } = {};");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "index_signature_any");
    }

    #[test]
    fn test_file_downgrades_to_info() {
        let findings = parse_and_check_path("let x: Record<string, any> = {};", "src/foo.test.ts");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn suppression_skips_record_any() {
        let findings = parse_and_check("// virgil-ignore\nlet x: Record<string, any> = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_index_signature() {
        let findings = parse_and_check("// @ts-ignore\nlet x: { [key: string]: any } = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        let pipeline = RecordStringAnyPipeline::new(Language::Tsx).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Tsx.tree_sitter_language())
            .unwrap();
        let tree = parser
            .parse("let x: Record<string, any> = {};", None)
            .unwrap();
        let findings = pipeline.check(&tree, b"let x: Record<string, any> = {};", "test.tsx");
        assert_eq!(findings.len(), 1);
    }
}
