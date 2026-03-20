use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{ControlFlowConfig, compute_cognitive};
use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

const QUERY_SRC: &str = r#"
[
  (function_definition
    name: (name) @fn_name
    body: (compound_statement) @fn_body) @func
  (method_declaration
    name: (name) @fn_name
    body: (compound_statement) @fn_body) @func
]
"#;

fn php_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "case_statement",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause", "else_if_clause"],
        logical_operators: &["&&", "||", "and", "or"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

pub struct CognitiveComplexityPipeline {
    query: Query,
}

impl CognitiveComplexityPipeline {
    pub fn new() -> Result<Self> {
        let lang = crate::language::Language::Php.tree_sitter_language();
        let query = Query::new(&lang, QUERY_SRC)?;
        Ok(Self { query })
    }
}

impl Pipeline for CognitiveComplexityPipeline {
    fn name(&self) -> &str {
        "cognitive_complexity"
    }

    fn description(&self) -> &str {
        "Measures cognitive complexity of PHP functions and methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "fn_name");
        let body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        let config = php_config();

        while let Some(m) = matches.next() {
            let name_cap = m.captures.iter().find(|c| c.index as usize == name_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);
            let func_cap = m.captures.iter().find(|c| c.index as usize == func_idx);

            if let (Some(name_cap), Some(body_cap), Some(func_cap)) = (name_cap, body_cap, func_cap)
            {
                let fn_name = node_text(name_cap.node, source);
                let cog = compute_cognitive(body_cap.node, &config, source);

                if cog > 15 {
                    let start = func_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "high_cognitive_complexity".to_string(),
                        message: format!(
                            "function `{fn_name}` has cognitive complexity {cog} (threshold: 15)"
                        ),
                        snippet: extract_snippet(source, func_cap.node, 3),
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CognitiveComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_high_cognitive_complexity() {
        // Deeply nested control flow to exceed threshold of 15
        let src = r#"<?php
function tangled($a, $b, $c, $d) {
    if ($a) {
        if ($b) {
            for ($i = 0; $i < 10; $i++) {
                if ($c) {
                    while ($d) {
                        if ($a && $b) {
                            break;
                        }
                    }
                }
            }
        }
    }
    if ($a || $b) {
        foreach ($c as $item) {
            if ($item) {
                return;
            }
        }
    }
}
?>"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].pattern, "high_cognitive_complexity");
    }

    #[test]
    fn clean_simple_function() {
        let src = "<?php\nfunction simple($x) { if ($x) { return 1; } return 0; }\n?>";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
