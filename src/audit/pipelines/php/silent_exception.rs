use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::php_primitives::{
    compile_catch_clause_query, extract_snippet, find_capture_index, node_text,
};

pub struct SilentExceptionPipeline {
    catch_query: Arc<Query>,
}

impl SilentExceptionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            catch_query: compile_catch_clause_query()?,
        })
    }
}

impl Pipeline for SilentExceptionPipeline {
    fn name(&self) -> &str {
        "silent_exception"
    }

    fn description(&self) -> &str {
        "Detects catch blocks that catch Exception but have empty or trivial bodies"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.catch_query, tree.root_node(), source);

        let catch_type_idx = find_capture_index(&self.catch_query, "catch_type");
        let catch_body_idx = find_capture_index(&self.catch_query, "catch_body");
        let catch_idx = find_capture_index(&self.catch_query, "catch");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_type_idx)
                .map(|c| c.node);
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

            if let (Some(type_node), Some(body_node), Some(catch_node)) =
                (type_node, body_node, catch_node)
            {
                // Check if catch type contains Exception or \Exception
                if !catches_exception(type_node, source) {
                    continue;
                }

                // Check if body is empty or only contains a return statement
                if !is_empty_or_trivial(body_node) {
                    continue;
                }

                let start = catch_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "silent_catch".to_string(),
                    message: "catch block silently swallows Exception — log or rethrow instead"
                        .to_string(),
                    snippet: extract_snippet(source, catch_node, 3),
                });
            }
        }

        findings
    }
}

fn catches_exception(type_node: tree_sitter::Node, source: &[u8]) -> bool {
    // Walk children of type_list looking for named_type nodes
    for i in 0..type_node.named_child_count() {
        if let Some(child) = type_node.named_child(i) {
            let type_text = node_text(child, source);
            if type_text == "Exception"
                || type_text == "\\Exception"
                || type_text == "\\Throwable"
                || type_text == "Throwable"
            {
                return true;
            }
        }
    }
    false
}

fn is_empty_or_trivial(body_node: tree_sitter::Node) -> bool {
    let named_count = body_node.named_child_count();
    if named_count == 0 {
        return true;
    }
    // If there's only one statement and it's a return, it's trivial
    if named_count == 1 {
        if let Some(child) = body_node.named_child(0) {
            return child.kind() == "return_statement";
        }
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SilentExceptionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_empty_catch() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "silent_catch");
    }

    #[test]
    fn detects_return_only_catch() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { return; }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_catch_with_logging() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { error_log($e->getMessage()); }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_specific_exception() {
        let src = "<?php\ntry { foo(); } catch (InvalidArgumentException $e) { }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_throwable_catch() {
        let src = "<?php\ntry { foo(); } catch (Throwable $e) { }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
