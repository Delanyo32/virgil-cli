use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const XML_PARSE_METHODS: &[(&str, &str)] = &[
    ("ET", "fromstring"),
    ("ET", "parse"),
    ("ElementTree", "fromstring"),
    ("ElementTree", "parse"),
    ("etree", "fromstring"),
    ("etree", "parse"),
    ("minidom", "parseString"),
    ("sax", "parseString"),
    ("sax", "parse"),
];

pub struct XxeFormatStringPipeline {
    call_query: Arc<Query>,
}

impl XxeFormatStringPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for XxeFormatStringPipeline {
    fn name(&self) -> &str {
        "xxe_format_string"
    }

    fn description(&self) -> &str {
        "Detects XXE risks from XML parsing with untrusted data, and format string injection"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(call_node)) =
                (fn_node, args_node, call_node)
                && fn_node.kind() == "attribute"
            {
                let obj = fn_node
                    .child_by_field_name("object")
                    .map(|n| node_text(n, source));
                let attr = fn_node
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source));

                if let (Some(obj_name), Some(attr_name)) = (obj, attr) {
                    // Check for XML parsing with untrusted data
                    let is_xml_parse = XML_PARSE_METHODS
                        .iter()
                        .any(|(m, f)| *m == obj_name && *f == attr_name);

                    if is_xml_parse
                        && let Some(first_arg) = args_node.named_child(0)
                        && first_arg.kind() != "string"
                    {
                        let start = call_node.start_position();
                        findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "warning".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "xxe_parse".to_string(),
                                        message: format!(
                                            "`{obj_name}.{attr_name}()` with dynamic input — use defusedxml to prevent XXE"
                                        ),
                                        snippet: extract_snippet(source, call_node, 1),
                                    });
                    }

                    // Check for format string injection: variable.format(...)
                    if attr_name == "format" {
                        // The object should be a variable, not a string literal
                        if fn_node.child_by_field_name("object").map(|n| n.kind()) != Some("string")
                        {
                            // This is variable.format(...) — only flag if the object is an identifier
                            // (user-controlled template)
                            if fn_node.child_by_field_name("object").map(|n| n.kind())
                                == Some("identifier")
                            {
                                let start = call_node.start_position();
                                findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "warning".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "format_string_injection".to_string(),
                                        message: format!(
                                            "`.format()` on variable `{obj_name}` — user-controlled format strings can leak data"
                                        ),
                                        snippet: extract_snippet(source, call_node, 1),
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
        let pipeline = XxeFormatStringPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_xxe_fromstring() {
        let src = "import xml.etree.ElementTree as ET\nET.fromstring(user_data)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "xxe_parse");
    }

    #[test]
    fn ignores_xxe_with_literal() {
        let src = "import xml.etree.ElementTree as ET\nET.fromstring(\"<root/>\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_format_string_injection() {
        let src = "template.format(secret=secret_value)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "format_string_injection");
    }

    #[test]
    fn ignores_literal_format() {
        let src = "\"Hello {}\".format(name)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
