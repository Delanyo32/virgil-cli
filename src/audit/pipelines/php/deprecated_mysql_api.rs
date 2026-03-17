use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct DeprecatedMysqlApiPipeline {
    call_query: Arc<Query>,
}

impl DeprecatedMysqlApiPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for DeprecatedMysqlApiPipeline {
    fn name(&self) -> &str {
        "deprecated_mysql_api"
    }

    fn description(&self) -> &str {
        "Detects calls to the deprecated mysql_* extension functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
                let fn_name = node_text(name_node, source);
                if fn_name.starts_with("mysql_") {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "deprecated_mysql_function".to_string(),
                        message: format!(
                            "`{fn_name}()` uses the removed mysql_* extension — migrate to mysqli or PDO"
                        ),
                        snippet: extract_snippet(source, call_node, 2),
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
        let pipeline = DeprecatedMysqlApiPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_mysql_connect() {
        let src = "<?php\nmysql_connect('localhost', 'root', '');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "deprecated_mysql_function");
        assert!(findings[0].message.contains("mysql_connect"));
    }

    #[test]
    fn detects_mysql_query() {
        let src = "<?php\nmysql_query('SELECT 1');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_mysqli() {
        let src = "<?php\nmysqli_connect('localhost', 'root', '');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_pdo() {
        let src = "<?php\n$pdo = new PDO('mysql:host=localhost', 'root', '');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
