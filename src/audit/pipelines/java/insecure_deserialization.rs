use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_query, extract_snippet, find_capture_index, node_text,
};

pub struct InsecureDeserializationPipeline {
    method_query: Arc<Query>,
}

impl InsecureDeserializationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_query()?,
        })
    }
}

impl Pipeline for InsecureDeserializationPipeline {
    fn name(&self) -> &str {
        "insecure_deserialization"
    }

    fn description(&self) -> &str {
        "Detects insecure deserialization via ObjectInputStream.readObject()"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.method_query, "method_name");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx).map(|c| c.node);
            let inv_node = m.captures.iter().find(|c| c.index as usize == inv_idx).map(|c| c.node);

            if let (Some(method_node), Some(inv_node)) = (method_node, inv_node) {
                let method_name = node_text(method_node, source);
                if method_name != "readObject" && method_name != "readUnshared" {
                    continue;
                }

                let inv_text = node_text(inv_node, source);
                // Heuristic: check if used with ObjectInputStream
                if inv_text.contains("ObjectInput") || inv_text.contains("ois.") || inv_text.contains("oin.") || inv_text.contains("objectInput") {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsafe_deserialization".to_string(),
                        message: format!(
                            "`.{method_name}()` deserializes untrusted data — potential remote code execution"
                        ),
                        snippet: extract_snippet(source, inv_node, 1),
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InsecureDeserializationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_read_object() {
        let src = r#"class Foo {
    void load(InputStream in) {
        ObjectInputStream ois = new ObjectInputStream(in);
        Object obj = ois.readObject();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_deserialization");
    }

    #[test]
    fn ignores_unrelated_read() {
        let src = r#"class Foo {
    void bar() {
        reader.readLine();
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_deserialization() {
        let src = r#"class Foo {
    void bar() {
        String s = "hello";
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
