use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_object_creation_query, extract_snippet, find_capture_index, node_text,
};

const DANGEROUS_SERIALIZERS: &[(&str, &str)] = &[
    ("BinaryFormatter", "binary_formatter"),
    ("NetDataContractSerializer", "unsafe_serializer"),
    ("SoapFormatter", "unsafe_serializer"),
    ("LosFormatter", "unsafe_serializer"),
    ("ObjectStateFormatter", "unsafe_serializer"),
];

pub struct InsecureDeserializationPipeline {
    creation_query: Arc<Query>,
}

impl InsecureDeserializationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
        })
    }
}

impl Pipeline for InsecureDeserializationPipeline {
    fn name(&self) -> &str {
        "insecure_deserialization"
    }

    fn description(&self) -> &str {
        "Detects insecure deserialization: BinaryFormatter, SoapFormatter, and other dangerous serializers"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

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

                if let Some((_, pattern)) = DANGEROUS_SERIALIZERS
                    .iter()
                    .find(|(name, _)| *name == type_name)
                {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: format!(
                            "new {type_name}() is inherently unsafe — use JsonSerializer or XmlSerializer with known types"
                        ),
                        snippet: extract_snippet(source, creation_node, 1),
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InsecureDeserializationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_binary_formatter() {
        let src = r#"class Foo {
    void Deserialize(Stream s) {
        var bf = new BinaryFormatter();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "binary_formatter");
    }

    #[test]
    fn detects_soap_formatter() {
        let src = r#"class Foo {
    void Deserialize(Stream s) {
        var sf = new SoapFormatter();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_serializer");
    }

    #[test]
    fn ignores_json_serializer() {
        let src = r#"class Foo {
    void Deserialize() {
        var js = new JsonSerializer();
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_serializer() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
