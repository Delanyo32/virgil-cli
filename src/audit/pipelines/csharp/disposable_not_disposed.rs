use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_local_decl_query, extract_snippet, find_capture_index, node_text};

const DISPOSABLE_TYPES: &[&str] = &[
    "FileStream",
    "StreamReader",
    "StreamWriter",
    "HttpClient",
    "SqlConnection",
    "SqlCommand",
    "SqlDataReader",
    "MemoryStream",
    "BinaryReader",
    "BinaryWriter",
    "TcpClient",
    "UdpClient",
    "NetworkStream",
    "Process",
    "Timer",
    "Mutex",
    "Semaphore",
    "ManualResetEvent",
    "AutoResetEvent",
    "CancellationTokenSource",
];

pub struct DisposableNotDisposedPipeline {
    local_query: Arc<Query>,
}

impl DisposableNotDisposedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            local_query: compile_local_decl_query()?,
        })
    }
}

impl Pipeline for DisposableNotDisposedPipeline {
    fn name(&self) -> &str {
        "disposable_not_disposed"
    }

    fn description(&self) -> &str {
        "Detects IDisposable types created outside using statements"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.local_query, tree.root_node(), source);

        let var_type_idx = find_capture_index(&self.local_query, "var_type");
        let var_decl_idx = find_capture_index(&self.local_query, "var_decl");

        while let Some(m) = matches.next() {
            let type_node = m.captures.iter().find(|c| c.index as usize == var_type_idx).map(|c| c.node);
            let decl_node = m.captures.iter().find(|c| c.index as usize == var_decl_idx).map(|c| c.node);

            if let (Some(type_node), Some(decl_node)) = (type_node, decl_node) {
                let type_text = node_text(type_node, source);

                if !DISPOSABLE_TYPES.contains(&type_text) {
                    continue;
                }

                // Check if this declaration is inside a using_statement or using_declaration
                if is_inside_using(decl_node) {
                    continue;
                }

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_using".to_string(),
                    message: format!(
                        "`{type_text}` implements IDisposable but is not wrapped in a `using` statement \u{2014} resources may leak"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

fn is_inside_using(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "using_statement" | "using_declaration" => return true,
            "method_declaration" | "constructor_declaration" | "class_declaration" => return false,
            _ => {}
        }
        current = parent.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&Language::CSharp.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DisposableNotDisposedPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_disposable_without_using() {
        let src = r#"
class Foo {
    void Bar() {
        FileStream fs = new FileStream("test.txt");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_using");
    }

    #[test]
    fn clean_with_using_statement() {
        let src = r#"
class Foo {
    void Bar() {
        using (FileStream fs = new FileStream("test.txt")) {
            fs.Read();
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_disposable() {
        let src = r#"
class Foo {
    void Bar() {
        StringBuilder sb = new StringBuilder();
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"class Foo { void M() { HttpClient c = new HttpClient(); } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "disposable_not_disposed");
    }
}
