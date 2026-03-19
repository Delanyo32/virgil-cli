use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_with_object_query, extract_snippet, find_capture_index, node_text,
};

const XML_FACTORIES: &[&str] = &[
    "DocumentBuilderFactory",
    "SAXParserFactory",
    "XMLInputFactory",
];

pub struct XxePipeline {
    method_query: Arc<Query>,
}

impl XxePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_with_object_query()?,
        })
    }
}

impl Pipeline for XxePipeline {
    fn name(&self) -> &str {
        "xxe"
    }

    fn description(&self) -> &str {
        "Detects XXE risks: XML parser factories without secure configuration"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.method_query, "object");
        let method_idx = find_capture_index(&self.method_query, "method_name");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        // Collect XML factory instantiations and check if disallow-doctype-decl is set
        let source_str = std::str::from_utf8(source).unwrap_or("");
        let has_secure_config = source_str.contains("disallow-doctype-decl")
            || source_str.contains("DISALLOW_DOCTYPE_DECL")
            || source_str.contains("XMLConstants.ACCESS_EXTERNAL_DTD");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(obj_node), Some(method_node), Some(inv_node)) =
                (obj_node, method_node, inv_node)
            {
                let obj_name = node_text(obj_node, source);
                let method_name = node_text(method_node, source);

                // Detect XML factory instantiation without secure config
                if XML_FACTORIES.contains(&obj_name) && method_name == "newInstance" {
                    if !has_secure_config {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unsafe_xml_parser".to_string(),
                            message: format!(
                                "{obj_name}.newInstance() without disabling external entities — vulnerable to XXE"
                            ),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // Detect XPath injection via string concat
                if method_name == "evaluate" || method_name == "compile" {
                    let inv_text = node_text(inv_node, source);
                    if inv_text.contains("XPath") || inv_text.contains("xpath") {
                        if inv_text.contains('+') {
                            let start = inv_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "xpath_injection".to_string(),
                                message: "XPath expression built with string concatenation — use parameterized XPath".to_string(),
                                snippet: extract_snippet(source, inv_node, 1),
                            });
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = XxePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_unsafe_document_builder() {
        let src = r#"class Foo {
    void parse() {
        DocumentBuilderFactory.newInstance();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_xml_parser");
    }

    #[test]
    fn detects_unsafe_sax_parser() {
        let src = r#"class Foo {
    void parse() {
        SAXParserFactory.newInstance();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_xml_parser");
    }

    #[test]
    fn ignores_secure_factory() {
        let src = r#"class Foo {
    void parse() {
        DocumentBuilderFactory dbf = DocumentBuilderFactory.newInstance();
        dbf.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_xml() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
