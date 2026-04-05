use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_local_decl_query, extract_snippet, find_capture_index, is_csharp_suppressed, node_text,
};

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
    "DbContext",
    "HttpResponseMessage",
    "WebClient",
    "SmtpClient",
    "SemaphoreSlim",
    "Bitmap",
    "Graphics",
    "Font",
    "Pen",
    "Brush",
    "Channel",
    "EventWaitHandle",
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

impl GraphPipeline for DisposableNotDisposedPipeline {
    fn name(&self) -> &str {
        "disposable_not_disposed"
    }

    fn description(&self) -> &str {
        "Detects IDisposable types created outside using statements"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.local_query, tree.root_node(), source);

        let var_type_idx = find_capture_index(&self.local_query, "var_type");
        let var_name_idx = find_capture_index(&self.local_query, "var_name");
        let var_decl_idx = find_capture_index(&self.local_query, "var_decl");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_type_idx)
                .map(|c| c.node);
            let _name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_name_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_decl_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(decl_node)) = (type_node, decl_node) {
                let type_text = node_text(type_node, source);

                // Check explicit type or `var` with known-disposable `new Type(...)`
                let is_disposable = if DISPOSABLE_TYPES.contains(&type_text) {
                    true
                } else if type_text == "var" {
                    // Check if RHS is `new KnownDisposableType(...)`
                    get_rhs_type(decl_node, source)
                        .is_some_and(|rhs| DISPOSABLE_TYPES.contains(&rhs))
                } else {
                    false
                };

                if !is_disposable {
                    continue;
                }

                if is_inside_using(decl_node) {
                    continue;
                }

                if is_field_assignment(decl_node, source) {
                    continue;
                }

                if is_csharp_suppressed(source, decl_node, "disposable_not_disposed") {
                    continue;
                }

                let display_type = if type_text == "var" {
                    get_rhs_type(decl_node, source).unwrap_or("IDisposable")
                } else {
                    type_text
                };

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_using".to_string(),
                    message: format!(
                        "`{display_type}` implements IDisposable but is not wrapped in a `using` statement \u{2014} resources may leak"
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

fn is_field_assignment(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "assignment_expression" {
            if let Some(left) = parent.child_by_field_name("left") {
                let text = left.utf8_text(source).unwrap_or("");
                if text.starts_with("this.") || text.starts_with("_") {
                    return true;
                }
            }
        }
        if parent.kind() == "method_declaration" || parent.kind() == "class_declaration" {
            break;
        }
        current = parent.parent();
    }
    false
}

/// Extract the type name from `new TypeName(...)` on the RHS of a variable declaration.
fn get_rhs_type<'a>(
    decl_node: tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<&'a str> {
    // Walk into variable_declaration > variable_declarator > equals_value_clause > object_creation_expression
    let mut stack = vec![decl_node];
    while let Some(n) = stack.pop() {
        if n.kind() == "object_creation_expression" {
            if let Some(type_node) = n.child_by_field_name("type") {
                return Some(node_text(type_node, source));
            }
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Service.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DisposableNotDisposedPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn test_file_excluded() {
        let src = r#"
class Foo {
    void Bar() {
        FileStream fs = new FileStream("test.txt");
    }
}
"#;
        let findings = parse_and_check_with_path(src, "FooTests.cs");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppressed_by_nolint() {
        let src = r#"
class Foo {
    void Bar() {
        // NOLINT
        FileStream fs = new FileStream("test.txt");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn expanded_type_detected() {
        let src = r#"
class Foo {
    void Bar() {
        DbContext ctx = new DbContext();
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn var_with_known_disposable_detected() {
        let src = r#"
class Foo {
    void Bar() {
        var fs = new FileStream("test.txt");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FileStream"));
    }
}
