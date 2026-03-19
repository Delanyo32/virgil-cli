use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_invocation_query, compile_object_creation_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct XxePipeline {
    creation_query: Arc<Query>,
    invocation_query: Arc<Query>,
}

impl XxePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
            invocation_query: compile_invocation_query()?,
        })
    }
}

impl Pipeline for XxePipeline {
    fn name(&self) -> &str {
        "xxe"
    }

    fn description(&self) -> &str {
        "Detects XXE risks: XmlDocument and XmlTextReader without secure configuration"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_xml_creation(tree, source, file_path, &mut findings);
        self.check_xpath_injection(tree, source, file_path, &mut findings);
        findings
    }
}

impl XxePipeline {
    fn check_xml_creation(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

        let source_str = std::str::from_utf8(source).unwrap_or("");
        let has_xml_resolver_null =
            source_str.contains("XmlResolver = null") || source_str.contains("XmlResolver=null");
        let has_dtd_prohibit = source_str.contains("DtdProcessing.Prohibit")
            || source_str.contains("ProhibitDtd = true");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(creation_node)) = (type_node, creation_node) {
                let type_name = node_text(type_node, source);

                if type_name == "XmlDocument" && !has_xml_resolver_null {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsafe_xml_document".to_string(),
                        message: "new XmlDocument() without XmlResolver = null — vulnerable to XXE"
                            .to_string(),
                        snippet: extract_snippet(source, creation_node, 1),
                    });
                }

                if type_name == "XmlTextReader" && !has_dtd_prohibit {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsafe_xml_reader".to_string(),
                        message:
                            "new XmlTextReader() without DtdProcessing.Prohibit — vulnerable to XXE"
                                .to_string(),
                        snippet: extract_snippet(source, creation_node, 1),
                    });
                }
            }
        }
    }

    fn check_xpath_injection(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let inv_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(inv_node)) = (fn_node, args_node, inv_node)
            {
                let fn_text = node_text(fn_node, source);
                if !fn_text.contains("SelectNodes")
                    && !fn_text.contains("SelectSingleNode")
                    && !fn_text.contains("Evaluate")
                {
                    continue;
                }

                let args_text = node_text(args_node, source);
                if args_text.contains('+') || contains_interpolation(args_node) {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "xpath_injection".to_string(),
                        message:
                            "XPath expression built with dynamic input — use parameterized XPath"
                                .to_string(),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }
            }
        }
    }
}

fn contains_interpolation(node: tree_sitter::Node) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == "interpolated_string_expression" {
            return true;
        }
        for i in 0..current.named_child_count() {
            if let Some(child) = current.named_child(i) {
                stack.push(child);
            }
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = XxePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_unsafe_xml_document() {
        let src = r#"class Foo {
    void Parse(string xml) {
        var doc = new XmlDocument();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_xml_document");
    }

    #[test]
    fn detects_unsafe_xml_reader() {
        let src = r#"class Foo {
    void Parse(string xml) {
        var reader = new XmlTextReader(stream);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_xml_reader");
    }

    #[test]
    fn ignores_secure_xml_document() {
        let src = r#"class Foo {
    void Parse(string xml) {
        var doc = new XmlDocument();
        doc.XmlResolver = null;
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_xml() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
