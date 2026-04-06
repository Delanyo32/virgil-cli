use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{
    is_nolint_suppressed, COMMON_ALLOWED_NUMBERS, is_test_context_rust, is_test_file,
};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", // Common powers of 2 and sizes
    "10", "100", "1000", "256", "512", "1024", "2048", "4096", "8192", "16384", "32768", "65536",
    // Common hex masks
    "0xFF", "0xff", "0x80", "0xFFFF", "0xffff", "0xFF00", "0xff00", "0x00", "0x01", "0x02",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "const_item",
    "static_item",
    "enum_variant",
    "attribute_item",
    "match_arm",
    "range_expression",
    "macro_invocation",
    "token_tree", // fixes numbers inside macro arguments (e.g. println!("{}", 9999))
];

pub struct MagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl MagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: primitives::compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if EXEMPT_ANCESTOR_KINDS.contains(&parent.kind()) {
                return true;
            }
            current = parent.parent();
        }

        // Skip if this is an index expression (arr[0])
        if let Some(parent) = node.parent()
            && parent.kind() == "index_expression"
        {
            // Check if this literal is the index (second child)
            if let Some(index_child) = parent.named_child(1)
                && index_child.id() == node.id()
            {
                return true;
            }
        }

        false
    }
}

impl GraphPipeline for MagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals not in const/static/enum contexts that should be named constants"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let file_path = ctx.file_path;
        let source = ctx.source;
        let tree = ctx.tree;

        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let number_idx = self
            .numeric_query
            .capture_names()
            .iter()
            .position(|n| *n == "number")
            .unwrap();

        // First pass: collect all candidate (value, line, col, snippet) tuples
        let mut candidates: Vec<(String, u32, u32, String)> = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let num_node = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_node {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) || COMMON_ALLOWED_NUMBERS.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node) {
                    continue;
                }

                // Skip numbers inside test contexts
                if is_test_context_rust(num_cap.node, source) {
                    continue;
                }

                // NOLINT suppression check
                if is_nolint_suppressed(source, num_cap.node, self.name()) {
                    continue;
                }

                let start = num_cap.node.start_position();
                candidates.push((
                    value.to_string(),
                    start.row as u32 + 1,
                    start.column as u32 + 1,
                    value.to_string(),
                ));
            }
        }

        // Second pass: compute frequency map (owned keys so we can consume candidates after)
        let mut freq: HashMap<String, usize> = HashMap::new();
        for (value, _, _, _) in &candidates {
            *freq.entry(value.clone()).or_insert(0) += 1;
        }

        // Emit findings with severity based on frequency
        candidates
            .into_iter()
            .map(|(value, line, column, snippet)| {
                let count = freq.get(&value).copied().unwrap_or(1);
                let severity = if count >= 3 { "warning" } else { "info" };
                AuditFinding {
                    file_path: file_path.to_string(),
                    line,
                    column,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_number".to_string(),
                    message: format!(
                        "magic number `{value}` — consider extracting to a named constant for clarity"
                    ),
                    snippet,
                }
            })
            .collect()
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
        let pipeline = MagicNumbersPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.rs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_magic_number() {
        let src = r#"
fn example() {
    let x = 9999;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("9999"));
    }

    #[test]
    fn skips_const_context() {
        let src = r#"
const N: usize = 1024;
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = r#"
fn example() {
    let x = 1;
    let y = 0;
    let z = 2;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_index_expression() {
        let src = r#"
fn example() {
    let x = arr[0];
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_static_context() {
        let src = r#"
static MAX: usize = 512;
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_float_magic_number() {
        let src = r#"
fn example() {
    let pi = 3.14159;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }

    #[test]
    fn number_in_macro_args_not_flagged() {
        let src = r#"fn f() { println!("{}", 9999); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "numbers in macro token_tree should be exempt");
    }

    #[test]
    fn assert_eq_args_not_flagged() {
        let src = r#"fn f() { assert_eq!(result, 42); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "numbers inside assert_eq! should be exempt");
    }

    #[test]
    fn nolint_suppresses_magic_number() {
        let src = "fn f() {\n    let x = 9999; // NOLINT(magic_numbers)\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "NOLINT comment should suppress");
    }

    #[test]
    fn repeated_value_is_warning() {
        let src = r#"
fn a() { let x = 9999; }
fn b() { let y = 9999; }
fn c() { let z = 9999; }
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert!(
            findings.iter().all(|f| f.severity == "warning"),
            "value appearing 3+ times should all be warning"
        );
    }

    #[test]
    fn single_occurrence_is_info() {
        let src = r#"fn f() { let x = 9999; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }
}
