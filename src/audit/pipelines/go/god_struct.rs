use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_decl_query, compile_struct_type_query, extract_snippet, find_capture_index,
    node_text,
};

const FIELD_THRESHOLD: usize = 15;
const METHOD_THRESHOLD: usize = 10;
const DTO_TAGS: &[&str] = &["json:", "yaml:", "toml:", "db:", "gorm:"];
const CONFIG_SUFFIXES: &[&str] = &["Config", "Options", "Settings", "Params"];

pub struct GodStructPipeline {
    struct_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl GodStructPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            struct_query: compile_struct_type_query()?,
            method_query: compile_method_decl_query()?,
        })
    }
}

impl Pipeline for GodStructPipeline {
    fn name(&self) -> &str {
        "god_struct"
    }

    fn description(&self) -> &str {
        "Detects structs with too many fields (>=15) or too many methods (>=10)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check large structs
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.struct_query, tree.root_node(), source);

            let name_idx = find_capture_index(&self.struct_query, "struct_name");
            let fields_idx = find_capture_index(&self.struct_query, "fields");
            let decl_idx = find_capture_index(&self.struct_query, "type_decl");

            while let Some(m) = matches.next() {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == name_idx)
                    .map(|c| c.node);
                let fields_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fields_idx)
                    .map(|c| c.node);
                let decl_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == decl_idx)
                    .map(|c| c.node);

                if let (Some(name_node), Some(fields_node), Some(decl_node)) =
                    (name_node, fields_node, decl_node)
                {
                    let field_count = (0..fields_node.named_child_count())
                        .filter_map(|i| fields_node.named_child(i))
                        .filter(|child| child.kind() == "field_declaration")
                        .count();

                    if field_count >= FIELD_THRESHOLD {
                        let struct_name = node_text(name_node, source);

                        // Skip structs with Config/Options/Settings/Params suffix
                        if CONFIG_SUFFIXES.iter().any(|s| struct_name.ends_with(s)) {
                            continue;
                        }

                        // Skip DTO/model structs (field tags contain json:, yaml:, etc.)
                        let has_dto_tag = (0..fields_node.named_child_count())
                            .filter_map(|i| fields_node.named_child(i))
                            .filter(|child| child.kind() == "field_declaration")
                            .any(|field| {
                                let field_text = node_text(field, source);
                                DTO_TAGS.iter().any(|tag| field_text.contains(tag))
                            });
                        if has_dto_tag {
                            continue;
                        }

                        let start = decl_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "large_struct".to_string(),
                            message: format!(
                                "struct `{struct_name}` has {field_count} fields (threshold: {FIELD_THRESHOLD}) — consider splitting"
                            ),
                            snippet: extract_snippet(source, decl_node, 3),
                        });
                    }
                }
            }
        }

        // Check large method sets
        {
            let mut method_counts: HashMap<String, usize> = HashMap::new();
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

            let receiver_idx = find_capture_index(&self.method_query, "receiver_type");
            let method_decl_idx = find_capture_index(&self.method_query, "method_decl");

            // Track first method_decl node per struct for reporting
            let mut first_method: HashMap<String, (u32, u32, String)> = HashMap::new();

            while let Some(m) = matches.next() {
                let receiver_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == receiver_idx)
                    .map(|c| c.node);
                let decl_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_decl_idx)
                    .map(|c| c.node);

                if let (Some(receiver_node), Some(decl_node)) = (receiver_node, decl_node) {
                    let receiver_text = node_text(receiver_node, source);
                    // Strip pointer `*` and parens from receiver type
                    let struct_name = receiver_text
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim_start_matches('*')
                        .to_string();

                    *method_counts.entry(struct_name.clone()).or_insert(0) += 1;

                    first_method.entry(struct_name).or_insert_with(|| {
                        let start = decl_node.start_position();
                        (
                            start.row as u32 + 1,
                            start.column as u32 + 1,
                            extract_snippet(source, decl_node, 1),
                        )
                    });
                }
            }

            for (struct_name, count) in &method_counts {
                if *count >= METHOD_THRESHOLD {
                    let (line, column, snippet) = first_method.get(struct_name).unwrap();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: *line,
                        column: *column,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "large_method_set".to_string(),
                        message: format!(
                            "type `{struct_name}` has {count} methods (threshold: {METHOD_THRESHOLD}) — consider splitting responsibilities"
                        ),
                        snippet: snippet.clone(),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodStructPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    fn gen_fields(n: usize) -> String {
        (0..n)
            .map(|i| format!("\tField{i} int"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn gen_methods(struct_name: &str, n: usize) -> String {
        (0..n)
            .map(|i| format!("func (s *{struct_name}) Method{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn detects_large_struct() {
        let src = format!(
            "package main\ntype BigStruct struct {{\n{}\n}}\n",
            gen_fields(16)
        );
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_struct");
    }

    #[test]
    fn clean_small_struct() {
        let src = format!(
            "package main\ntype SmallStruct struct {{\n{}\n}}\n",
            gen_fields(5)
        );
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_large_method_set() {
        let src = format!(
            "package main\ntype Svc struct {{}}\n{}\n",
            gen_methods("Svc", 11)
        );
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_method_set");
    }

    #[test]
    fn clean_small_method_set() {
        let src = format!(
            "package main\ntype Svc struct {{}}\n{}\n",
            gen_methods("Svc", 3)
        );
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }
}
