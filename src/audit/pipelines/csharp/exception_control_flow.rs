use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_catch_clause_query, extract_snippet, find_capture_index, is_csharp_suppressed,
    is_pragma_suppressed, node_text,
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

impl GraphPipeline for ExceptionControlFlowPipeline {
    fn name(&self) -> &str {
        "exception_control_flow"
    }

    fn description(&self) -> &str {
        "Detects empty catch blocks, catch-return-default, and overly broad catch (Exception)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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
                if is_csharp_suppressed(source, catch_node, "exception_control_flow") {
                    continue;
                }

                let named_count = body_node.named_child_count();

                // Pattern 1: empty catch
                if named_count == 0 {
                    let start = catch_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "empty_catch".to_string(),
                        message: "empty catch block silently swallows exception \u{2014} log or rethrow instead".to_string(),
                        snippet: extract_snippet(source, catch_node, 3),
                    });
                    continue;
                }

                // Pattern 2: catch returning a default value (null, false, "", default)
                if named_count == 1 {
                    let child = body_node.named_child(0).unwrap();
                    if is_return_default(child, source) {
                        let start = catch_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "catch_return_default".to_string(),
                            message: "catch block returns a default value \u{2014} propagate the exception or return a meaningful result".to_string(),
                            snippet: extract_snippet(source, catch_node, 3),
                        });
                        continue;
                    }
                }

                // Pattern 3: overly broad catch (Exception / System.Exception)
                let catch_type = get_catch_type(catch_node, source);
                let is_broad = matches!(catch_type.as_deref(), Some("Exception" | "System.Exception"));
                // Also detect bare `catch` without any type (even broader)
                let is_bare_catch = catch_type.is_none() && !has_catch_declaration(catch_node);

                if is_broad || is_bare_catch {
                    // Skip if has `catch ... when (...)` filter — it narrows the catch
                    if has_catch_filter(catch_node) {
                        continue;
                    }

                    // Skip if body contains `throw;` (rethrow) — this is legitimate logging+rethrow
                    if body_has_rethrow(body_node, source) {
                        continue;
                    }

                    // Check for #pragma warning disable CA1031
                    if is_pragma_suppressed(source, catch_node, &["CA1031"]) {
                        continue;
                    }

                    let pattern_name = if is_bare_catch {
                        "bare_catch"
                    } else {
                        "overly_broad_catch"
                    };
                    let msg = if is_bare_catch {
                        "bare `catch` without exception type catches everything \u{2014} specify an exception type"
                    } else {
                        "catching base `Exception` is too broad \u{2014} catch specific exception types instead"
                    };

                    let start = catch_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern_name.to_string(),
                        message: msg.to_string(),
                        snippet: extract_snippet(source, catch_node, 3),
                    });
                }
            }
        }

        findings
    }
}

/// Check if a return statement returns a default-like value: null, false, -1, "", default, default(T).
fn is_return_default(node: tree_sitter::Node, source: &[u8]) -> bool {
    if node.kind() != "return_statement" {
        return false;
    }
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            match child.kind() {
                "null_literal" => return true,
                "boolean_literal" => {
                    let text = node_text(child, source);
                    if text == "false" {
                        return true;
                    }
                }
                "string_literal" => {
                    let text = node_text(child, source);
                    if text == "\"\"" {
                        return true;
                    }
                }
                "prefix_unary_expression" | "integer_literal" => {
                    let text = node_text(child, source);
                    if text == "-1" {
                        return true;
                    }
                }
                "default_expression" => return true,
                _ => {
                    let text = node_text(child, source);
                    if text == "default" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn has_catch_declaration(catch_node: tree_sitter::Node) -> bool {
    let mut cursor = catch_node.walk();
    catch_node
        .children(&mut cursor)
        .any(|child| child.kind() == "catch_declaration")
}

fn get_catch_type(catch_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = catch_node.walk();
    let declaration = catch_node
        .children(&mut cursor)
        .find(|child| child.kind() == "catch_declaration")?;
    let mut decl_cursor = declaration.walk();
    for child in declaration.children(&mut decl_cursor) {
        if child.kind() == "identifier" || child.kind() == "qualified_name" {
            return Some(node_text(child, source).to_string());
        }
    }
    None
}

/// Check if catch clause has a `when` filter clause.
fn has_catch_filter(catch_node: tree_sitter::Node) -> bool {
    let mut cursor = catch_node.walk();
    catch_node
        .children(&mut cursor)
        .any(|child| child.kind() == "catch_filter_clause")
}

/// Check if the catch body contains a bare `throw;` statement (rethrow).
fn body_has_rethrow(body_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut stack = vec![body_node];
    while let Some(node) = stack.pop() {
        if node.kind() == "throw_statement" {
            let text = node.utf8_text(source).unwrap_or("").trim();
            // Bare `throw;` (rethrow) vs `throw new ...` (new exception)
            if text == "throw;" {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExceptionControlFlowPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Service.cs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_empty_catch() {
        let src = "class Foo { void M() { try { } catch (Exception e) { } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_catch");
        assert_eq!(findings[0].severity, "error");
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
        assert_eq!(catch_null[0].severity, "warning");
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
        assert_eq!(broad[0].severity, "info");
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

    #[test]
    fn broad_catch_with_rethrow_excluded() {
        let src = r#"
class Foo {
    void M() {
        try { }
        catch (Exception e) {
            logger.Error(e);
            throw;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_catch_return_false() {
        let src = "class Foo { bool M() { try { return true; } catch (Exception e) { return false; } } }";
        let findings = parse_and_check(src);
        let defaults: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "catch_return_default")
            .collect();
        assert_eq!(defaults.len(), 1);
    }

    #[test]
    fn detects_system_exception_qualified() {
        let src = r#"
class Foo {
    void M() {
        try { }
        catch (System.Exception e) {
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
    fn suppressed_by_nolint() {
        let src = r#"
class Foo {
    void M() {
        try { }
        // NOLINT
        catch (Exception e) { }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
