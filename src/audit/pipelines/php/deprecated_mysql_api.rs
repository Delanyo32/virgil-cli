use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

/// The exact set of deprecated mysql_* extension functions removed in PHP 7.0.
const DEPRECATED_MYSQL_FUNCTIONS: &[&str] = &[
    "mysql_connect",
    "mysql_pconnect",
    "mysql_close",
    "mysql_select_db",
    "mysql_query",
    "mysql_unbuffered_query",
    "mysql_db_query",
    "mysql_list_dbs",
    "mysql_list_tables",
    "mysql_list_fields",
    "mysql_list_processes",
    "mysql_error",
    "mysql_errno",
    "mysql_affected_rows",
    "mysql_insert_id",
    "mysql_result",
    "mysql_num_rows",
    "mysql_num_fields",
    "mysql_fetch_row",
    "mysql_fetch_array",
    "mysql_fetch_assoc",
    "mysql_fetch_object",
    "mysql_data_seek",
    "mysql_fetch_lengths",
    "mysql_fetch_field",
    "mysql_field_seek",
    "mysql_free_result",
    "mysql_field_name",
    "mysql_field_table",
    "mysql_field_len",
    "mysql_field_type",
    "mysql_field_flags",
    "mysql_escape_string",
    "mysql_real_escape_string",
    "mysql_stat",
    "mysql_thread_id",
    "mysql_client_encoding",
    "mysql_ping",
    "mysql_get_client_info",
    "mysql_get_host_info",
    "mysql_get_proto_info",
    "mysql_get_server_info",
    "mysql_info",
    "mysql_set_charset",
    "mysql_db_name",
    "mysql_tablename",
    "mysql_create_db",
    "mysql_drop_db",
];

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

impl NodePipeline for DeprecatedMysqlApiPipeline {
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

                if !DEPRECATED_MYSQL_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                if is_nolint_suppressed(source, call_node, self.name()) {
                    continue;
                }

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

    // --- New tests ---

    #[test]
    fn skips_user_defined_mysql_prefix() {
        let src = "<?php\nfunction mysql_audit_helper() {}\nmysql_audit_helper();\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "user-defined mysql_audit_helper should not be flagged"
        );
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "<?php\n// NOLINT(deprecated_mysql_api)\nmysql_connect('localhost', 'root', '');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_mysql_fetch_assoc() {
        let src = "<?php\nmysql_fetch_assoc($result);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("mysql_fetch_assoc"));
    }
}
