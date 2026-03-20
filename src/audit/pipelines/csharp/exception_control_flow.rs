use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_catch_clause_query, extract_snippet, find_capture_index, node_text,
};

pub struct ExceptionControlFlowPipeline {
    catch_query: Arc<Query>,
}

impl ExceptionControlFlowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            catch_query: compile_catch_clause_query()?,
        })
    }
}

impl Pipeline for ExceptionControlFlowPipeline {
    fn name(&self) -> &str {
        "exception_control_flow"
    }

    fn description(&self) -> &str {
        "Detects empty catch blocks, catch-return-null, and overly broad catch (Exception)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.catch_query, tree.root_node(), source);

        let catch_body_idx = find_capture_index(&self.catch_query, "catch_body");
        let catch_idx = find_capture_index(&self.catch_query, "catch");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_body_idx)
                .map(|c| c.node);
            let catch_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_idx)
                .map(|c| c.node);

            if let (Some(body_node), Some(catch_node)) = (body_node, catch_node) {
                let named_count = body_node.named_child_count();

                // Pattern 1: empty catch
                if named_count == 0 {
                    let start = catch_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "empty_catch".to_string(),
                        message: "empty catch block silently swallows exception \u{2014} log or rethrow instead".to_string(),
                        snippet: extract_snippet(source, catch_node, 3),
                    });
                    continue;
                }

                // Pattern 2: catch returning null
                if named_count == 1 {
                    let child = body_node.named_child(0).unwrap();
                    if is_return_null(child) {
                        let start = catch_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "catch_return_default".to_string(),
                            message: "catch block returns null \u{2014} propagate the exception or return a meaningful default".to_string(),
                            snippet: extract_snippet(source, catch_node, 3),
                        });
                        continue;
                    }
                }

                // Pattern 3: overly broad catch (Exception)
                if let Some(declaration) = get_catch_declaration(catch_node) {
                    let type_text = get_catch_type_text(declaration, source);
                    if type_text == "Exception" {
                        let start = catch_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "overly_broad_catch".to_string(),
                            message: "catching base `Exception` is too broad \u{2014} catch specific exception types instead".to_string(),
                            snippet: extract_snippet(source, catch_node, 3),
                        });
                    }
                }
            }
        }

        findings
    }
}

fn is_return_null(node: tree_sitter::Node) -> bool {
    if node.kind() != "return_statement" {
        return false;
    }
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i)
            && child.kind() == "null_literal"
        {
            return true;
        }
    }
    false
}

fn get_catch_declaration(catch_node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = catch_node.walk();
    catch_node
        .children(&mut cursor)
        .find(|&child| child.kind() == "catch_declaration")
}

fn get_catch_type_text<'a>(declaration: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
    // catch_declaration contains a type child
    let mut cursor = declaration.walk();
    for child in declaration.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "qualified_name" {
            return node_text(child, source);
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExceptionControlFlowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_empty_catch() {
        let src = "class Foo { void M() { try { } catch (Exception e) { } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_catch");
    }

    #[test]
    fn detects_catch_return_null() {
        let src = "class Foo { object M() { try { return new object(); } catch (Exception e) { return null; } } }";
        let findings = parse_and_check(src);
        let catch_null: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "catch_return_default")
            .collect();
        assert_eq!(catch_null.len(), 1);
    }

    #[test]
    fn detects_broad_catch() {
        let src = r#"
class Foo {
    void M() {
        try { }
        catch (Exception e) {
            Console.WriteLine(e);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        let broad: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "overly_broad_catch")
            .collect();
        assert_eq!(broad.len(), 1);
    }

    #[test]
    fn clean_specific_catch() {
        let src = r#"
class Foo {
    void M() {
        try { }
        catch (InvalidOperationException e) {
            Console.WriteLine(e);
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_catch_with_logging() {
        let src = r#"
class Foo {
    void M() {
        try { }
        catch (ArgumentException e) {
            logger.Error(e);
            throw;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
