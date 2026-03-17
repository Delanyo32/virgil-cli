use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_member_access_query, compile_return_null_query, extract_snippet, find_capture_index,
};

pub struct NullReferenceRiskPipeline {
    return_null_query: Arc<Query>,
    member_access_query: Arc<Query>,
}

impl NullReferenceRiskPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            return_null_query: compile_return_null_query()?,
            member_access_query: compile_member_access_query()?,
        })
    }
}

impl Pipeline for NullReferenceRiskPipeline {
    fn name(&self) -> &str {
        "null_reference_risk"
    }

    fn description(&self) -> &str {
        "Detects explicit null returns and deep member access chains without null-conditional operators"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: explicit null returns
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.return_null_query, tree.root_node(), source);
            let return_stmt_idx = find_capture_index(&self.return_null_query, "return_stmt");

            while let Some(m) = matches.next() {
                let return_node = m.captures.iter().find(|c| c.index as usize == return_stmt_idx).map(|c| c.node);
                if let Some(return_node) = return_node {
                    let start = return_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "explicit_null_return".to_string(),
                        message: "explicit `return null` \u{2014} consider returning a default, throwing, or using nullable reference types".to_string(),
                        snippet: extract_snippet(source, return_node, 3),
                    });
                }
            }
        }

        // Pattern 2: deep member access chains (nested member_access_expression without ?.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.member_access_query, tree.root_node(), source);
            let member_access_idx = find_capture_index(&self.member_access_query, "member_access");

            while let Some(m) = matches.next() {
                let access_node = m.captures.iter().find(|c| c.index as usize == member_access_idx).map(|c| c.node);
                if let Some(access_node) = access_node {
                    // Count nesting depth: how many member_access_expression ancestors
                    let depth = count_member_access_depth(access_node);
                    if depth >= 3 {
                        // Check that the chain doesn't use conditional_access_expression (?.)
                        if !has_conditional_access_ancestor(access_node) {
                            // Only report on the outermost (deepest) chain
                            if access_node.parent().map_or(true, |p| p.kind() != "member_access_expression") {
                                let start = access_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "deep_member_chain".to_string(),
                                    message: format!(
                                        "deep member access chain (depth {depth}) without null-conditional operator (?.) \u{2014} risk of NullReferenceException"
                                    ),
                                    snippet: extract_snippet(source, access_node, 3),
                                });
                            }
                        }
                    }
                }
            }
        }

        findings
    }
}

fn count_member_access_depth(node: tree_sitter::Node) -> usize {
    let mut depth = 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "member_access_expression" {
            depth = depth.max(1 + count_member_access_depth(child));
        }
    }
    depth
}

fn has_conditional_access_ancestor(node: tree_sitter::Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "conditional_access_expression" {
            return true;
        }
        current = n.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&Language::CSharp.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NullReferenceRiskPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_return_null() {
        let src = r#"
class Foo {
    object Bar() {
        return null;
    }
}
"#;
        let findings = parse_and_check(src);
        let null_returns: Vec<_> = findings.iter().filter(|f| f.pattern == "explicit_null_return").collect();
        assert_eq!(null_returns.len(), 1);
    }

    #[test]
    fn detects_deep_chain() {
        let src = r#"
class Foo {
    void Bar() {
        var x = a.b.c.d;
    }
}
"#;
        let findings = parse_and_check(src);
        let chains: Vec<_> = findings.iter().filter(|f| f.pattern == "deep_member_chain").collect();
        assert_eq!(chains.len(), 1);
    }

    #[test]
    fn clean_short_chain() {
        let src = r#"
class Foo {
    void Bar() {
        var x = a.b;
    }
}
"#;
        let findings = parse_and_check(src);
        let chains: Vec<_> = findings.iter().filter(|f| f.pattern == "deep_member_chain").collect();
        assert!(chains.is_empty());
    }

    #[test]
    fn clean_no_null_return() {
        let src = r#"
class Foo {
    int Bar() {
        return 42;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
