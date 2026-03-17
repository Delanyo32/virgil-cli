use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const RE_METHODS: &[&str] = &["compile", "match", "search", "findall", "finditer", "sub", "fullmatch"];

pub struct ResourceExhaustionPipeline {
    call_query: Arc<Query>,
}

impl ResourceExhaustionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for ResourceExhaustionPipeline {
    fn name(&self) -> &str {
        "resource_exhaustion"
    }

    fn description(&self) -> &str {
        "Detects ReDoS risks: regex patterns with nested quantifiers"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m.captures.iter().find(|c| c.index as usize == fn_expr_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(call_node)) = (fn_node, args_node, call_node) {
                if fn_node.kind() != "attribute" {
                    continue;
                }

                let obj = fn_node.child_by_field_name("object").map(|n| node_text(n, source));
                let attr = fn_node.child_by_field_name("attribute").map(|n| node_text(n, source));

                if obj != Some("re") {
                    continue;
                }
                if let Some(method) = attr {
                    if !RE_METHODS.contains(&method) {
                        continue;
                    }
                } else {
                    continue;
                }

                // Check the first argument (the regex pattern)
                if let Some(first_arg) = args_node.named_child(0) {
                    let arg_text = node_text(first_arg, source);
                    if has_nested_quantifier(arg_text) {
                        let start = call_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "redos_pattern".to_string(),
                            message: "regex pattern with nested quantifiers — potential ReDoS".to_string(),
                            snippet: extract_snippet(source, call_node, 1),
                        });
                    }
                }
            }
        }

        findings
    }
}

/// Checks for nested quantifier patterns that can cause catastrophic backtracking.
/// Looks for patterns like `(a+)+`, `(a*)*`, `([^x]+)*`, `(a{2,})+`
fn has_nested_quantifier(pattern: &str) -> bool {
    // Simple heuristic: look for a group with a quantifier followed by another quantifier
    // Pattern: (...+|...*|...{n,})(+|*|{n,})
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            // Find matching closing paren
            let mut depth = 1;
            let mut j = i + 1;
            let mut has_inner_quantifier = false;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'+' | b'*' if depth == 1 => has_inner_quantifier = true,
                    b'{' if depth == 1 => has_inner_quantifier = true,
                    _ => {}
                }
                j += 1;
            }
            // j is now past the closing paren
            if has_inner_quantifier && j < bytes.len() {
                match bytes[j] {
                    b'+' | b'*' | b'{' => return true,
                    _ => {}
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ResourceExhaustionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_nested_quantifier() {
        let src = "import re\nre.compile(r\"(a+)+b\")";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "redos_pattern");
    }

    #[test]
    fn detects_nested_star() {
        let src = "import re\nre.search(r\"([^x]+)*y\", text)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ignores_simple_pattern() {
        let src = "import re\nre.compile(r\"^[a-z]+$\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_re_call() {
        let src = "foo.compile(r\"(a+)+\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_nested_quantifier_heuristic() {
        assert!(has_nested_quantifier(r"(a+)+"));
        assert!(has_nested_quantifier(r"(a*)*"));
        assert!(has_nested_quantifier(r"([^x]+)*"));
        assert!(!has_nested_quantifier(r"^[a-z]+$"));
        assert!(!has_nested_quantifier(r"(abc)+"));
    }
}
