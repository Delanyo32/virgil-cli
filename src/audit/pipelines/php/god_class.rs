use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::{is_nolint_suppressed, is_test_file};

use super::primitives::{compile_class_decl_query, extract_snippet, find_capture_index, node_text};

const METHOD_THRESHOLD: usize = 10;
/// Extra methods allowed per trait use statement (traits add methods invisibly).
const TRAIT_USE_ALLOWANCE: usize = 3;
/// Secondary threshold: if method count > 7 AND property count > this, flag as god class.
const PROPERTY_THRESHOLD: usize = 10;
const METHOD_THRESHOLD_WITH_PROPERTIES: usize = 7;

pub struct GodClassPipeline {
    class_query: Arc<Query>,
}

impl GodClassPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

impl NodePipeline for GodClassPipeline {
    fn name(&self) -> &str {
        "god_class"
    }

    fn description(&self) -> &str {
        "Detects classes with too many methods (>10), indicating a need to split responsibilities"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_decl_idx = find_capture_index(&self.class_query, "class_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(decl_node)) =
                (name_node, body_node, decl_node)
            {
                let class_name = node_text(name_node, source);

                let mut method_count = 0;
                let mut property_count = 0;
                let mut trait_use_count = 0;

                for i in 0..body_node.named_child_count() {
                    if let Some(child) = body_node.named_child(i) {
                        match child.kind() {
                            "method_declaration" => method_count += 1,
                            "property_declaration" => property_count += 1,
                            "use_declaration" => trait_use_count += 1,
                            _ => {}
                        }
                    }
                }

                // Adjust threshold based on trait uses
                let adjusted_threshold =
                    METHOD_THRESHOLD + (trait_use_count * TRAIT_USE_ALLOWANCE);

                // Primary: method count exceeds adjusted threshold
                // Secondary: many methods AND many properties (data-heavy god class)
                let is_god_class = method_count > adjusted_threshold
                    || (method_count > METHOD_THRESHOLD_WITH_PROPERTIES
                        && property_count > PROPERTY_THRESHOLD);

                if !is_god_class {
                    continue;
                }

                if is_nolint_suppressed(source, decl_node, self.name()) {
                    continue;
                }

                let mut detail = format!("{method_count} methods");
                if property_count > 0 {
                    detail.push_str(&format!(", {property_count} properties"));
                }
                if trait_use_count > 0 {
                    detail.push_str(&format!(
                        ", {trait_use_count} trait use(s) (threshold adjusted to {adjusted_threshold})"
                    ));
                }

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "god_class".to_string(),
                    message: format!(
                        "class `{class_name}` has {detail} — consider splitting responsibilities"
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
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.php")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodClassPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    fn gen_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    public function method{i}() {{}}\n"))
            .collect()
    }

    fn gen_properties(n: usize) -> String {
        (0..n)
            .map(|i| format!("    public $prop{i};\n"))
            .collect()
    }

    #[test]
    fn detects_god_class() {
        let methods = gen_methods(12);
        let src = format!("<?php\nclass BigClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "god_class");
        assert!(findings[0].message.contains("12 methods"));
    }

    #[test]
    fn clean_small_class() {
        let methods = gen_methods(3);
        let src = format!("<?php\nclass SmallClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn exactly_at_threshold_is_clean() {
        let methods = gen_methods(10);
        let src = format!("<?php\nclass EdgeClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    // --- New tests ---

    #[test]
    fn test_file_suppressed() {
        let methods = gen_methods(12);
        let src = format!("<?php\nclass BigClassTest {{\n{methods}}}\n");
        let findings = parse_and_check_path(&src, "tests/BigClassTest.php");
        assert!(findings.is_empty());
    }

    #[test]
    fn trait_use_adjusts_threshold() {
        // 12 methods + 1 trait use -> adjusted threshold = 10 + 3 = 13 -> 12 < 13, no finding
        let methods = gen_methods(12);
        let src = format!(
            "<?php\nclass ClassWithTrait {{\n    use SomeTrait;\n{methods}}}\n"
        );
        let findings = parse_and_check(&src);
        assert!(findings.is_empty(), "trait use should adjust threshold upward");
    }

    #[test]
    fn composite_threshold_many_properties() {
        // 8 methods + 12 properties -> exceeds composite threshold (>7 methods AND >10 properties)
        let methods = gen_methods(8);
        let properties = gen_properties(12);
        let src = format!("<?php\nclass DataHeavy {{\n{properties}{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1, "composite threshold should trigger");
    }

    #[test]
    fn nolint_suppresses_finding() {
        let methods = gen_methods(12);
        let src = format!("<?php\n// NOLINT(god_class)\nclass BigClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }
}
