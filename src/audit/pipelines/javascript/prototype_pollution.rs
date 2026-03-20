use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_for_in_query, compile_method_call_security_query, compile_subscript_assignment_query,
    extract_snippet, find_capture_index, node_text,
};

const DANGEROUS_KEYS: &[&str] = &["__proto__", "constructor", "prototype"];

pub struct PrototypePollutionPipeline {
    #[allow(dead_code)]
    subscript_assign_query: Arc<Query>,
    for_in_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl PrototypePollutionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            subscript_assign_query: compile_subscript_assignment_query(language)?,
            for_in_query: compile_for_in_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for PrototypePollutionPipeline {
    fn name(&self) -> &str {
        "prototype_pollution"
    }

    fn description(&self) -> &str {
        "Detects prototype pollution: recursive merge without blocklist, Object.assign with parsed JSON"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Detect target[key] = value inside for...in without __proto__/constructor guard
        self.check_for_in_merge(&mut findings, tree, source, file_path);

        // Detect Object.assign(target, JSON.parse(...))
        self.check_object_assign_json_parse(&mut findings, tree, source, file_path);

        findings
    }
}

impl PrototypePollutionPipeline {
    fn check_for_in_merge(
        &self,
        findings: &mut Vec<AuditFinding>,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.for_in_query, tree.root_node(), source);
        let body_idx = find_capture_index(&self.for_in_query, "body");
        let for_in_idx = find_capture_index(&self.for_in_query, "for_in");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let for_in_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == for_in_idx)
                .map(|c| c.node);

            if let (Some(body), Some(for_in)) = (body_node, for_in_node) {
                let body_text = node_text(body, source);

                // Check if body contains subscript assignment (target[key] = source[key])
                let has_subscript_assign = body_text.contains('[') && body_text.contains("] =");

                if !has_subscript_assign {
                    continue;
                }

                // Check if body contains a guard against dangerous keys
                let has_guard = DANGEROUS_KEYS.iter().any(|key| body_text.contains(key))
                    || body_text.contains("hasOwnProperty");

                if !has_guard {
                    let start = for_in.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "prototype_pollution_merge".to_string(),
                        message: "for...in loop with subscript assignment without __proto__/constructor guard — prototype pollution risk".to_string(),
                        snippet: extract_snippet(source, for_in, 3),
                    });
                }
            }
        }
    }

    fn check_object_assign_json_parse(
        &self,
        findings: &mut Vec<AuditFinding>,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
        let obj_idx = find_capture_index(&self.method_call_query, "obj");
        let method_idx = find_capture_index(&self.method_call_query, "method");
        let call_idx = find_capture_index(&self.method_call_query, "call");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(obj), Some(method), Some(call)) = (obj_node, method_node, call_node) {
                let obj_name = node_text(obj, source);
                let method_name = node_text(method, source);

                if obj_name == "Object" && method_name == "assign" {
                    let call_text = node_text(call, source);
                    if call_text.contains("JSON.parse") {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "object_assign_parsed_json".to_string(),
                            message: "`Object.assign()` with `JSON.parse()` source — prototype pollution risk".to_string(),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::JavaScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PrototypePollutionPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_for_in_merge_without_guard() {
        let src = r#"
function merge(target, source) {
    for (let key in source) {
        target[key] = source[key];
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "prototype_pollution_merge");
    }

    #[test]
    fn ignores_for_in_merge_with_proto_guard() {
        let src = r#"
function merge(target, source) {
    for (let key in source) {
        if (key === "__proto__") continue;
        target[key] = source[key];
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_for_in_with_has_own_property() {
        let src = r#"
function merge(target, source) {
    for (let key in source) {
        if (source.hasOwnProperty(key)) {
            target[key] = source[key];
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_object_assign_json_parse() {
        let src = "Object.assign(config, JSON.parse(userInput));";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "object_assign_parsed_json");
    }

    #[test]
    fn ignores_object_assign_with_literal() {
        let src = "Object.assign(config, { key: 1 });";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
