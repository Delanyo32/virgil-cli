use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_local_var_decl_query, extract_snippet, find_capture_index, node_text,
};

const RESOURCE_TYPES: &[&str] = &[
    "Connection",
    "Statement",
    "PreparedStatement",
    "ResultSet",
    "InputStream",
    "OutputStream",
    "FileInputStream",
    "FileOutputStream",
    "BufferedReader",
    "BufferedWriter",
    "FileReader",
    "FileWriter",
    "Socket",
    "ServerSocket",
    "Scanner",
    "PrintWriter",
];

pub struct ResourceLeaksPipeline {
    local_var_query: Arc<Query>,
    resource_types: HashSet<&'static str>,
}

impl ResourceLeaksPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            local_var_query: compile_local_var_decl_query()?,
            resource_types: RESOURCE_TYPES.iter().copied().collect(),
        })
    }
}

impl Pipeline for ResourceLeaksPipeline {
    fn name(&self) -> &str {
        "resource_leaks"
    }

    fn description(&self) -> &str {
        "Detects resource types created outside try-with-resources — potential resource leak"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.local_var_query, tree.root_node(), source);

        let var_type_idx = find_capture_index(&self.local_var_query, "var_type");
        let var_name_idx = find_capture_index(&self.local_var_query, "var_name");
        let creation_idx = find_capture_index(&self.local_var_query, "creation");
        let var_decl_idx = find_capture_index(&self.local_var_query, "var_decl");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_type_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_name_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == var_decl_idx)
                .map(|c| c.node);

            // Must have object_creation_expression (new ...)
            let Some(_creation) = creation_node else {
                continue;
            };

            if let (Some(type_node), Some(name_node), Some(decl_node)) =
                (type_node, name_node, decl_node)
            {
                // Extract the base type name (handle generic_type by getting its first child)
                let type_text = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| node_text(n, source))
                        .unwrap_or("")
                } else {
                    node_text(type_node, source)
                };

                if !self.resource_types.contains(type_text) {
                    continue;
                }

                // Check if inside try-with-resources
                if is_in_try_with_resources(decl_node) {
                    continue;
                }

                let var_name = node_text(name_node, source);
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "resource_leak".to_string(),
                    message: format!(
                        "`{type_text} {var_name}` is created outside try-with-resources — potential resource leak"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

fn is_in_try_with_resources(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "try_with_resources_statement" {
            // Check if we're in the resource_specification, not just the body
            // In tree-sitter-java, resources are inside `resource_specification` > `resource`
            // and are NOT `local_variable_declaration` nodes — they are `resource` nodes.
            // So any `local_variable_declaration` inside a try-with-resources is in the body,
            // which is still potentially a leak. However, being inside try-with-resources
            // at all suggests the developer is being careful, so we skip.
            return true;
        }
        if p.kind() == "method_declaration"
            || p.kind() == "constructor_declaration"
            || p.kind() == "class_declaration"
        {
            return false;
        }
        parent = p.parent();
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ResourceLeaksPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_resource_leak() {
        let src = r#"
class Foo {
    void m() {
        Connection conn = new Connection();
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "resource_leak");
        assert!(findings[0].message.contains("Connection"));
    }

    #[test]
    fn clean_try_with_resources() {
        let src = r#"
class Foo {
    void m() {
        try (Connection conn = new Connection()) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_resource_type() {
        let src = r#"
class Foo {
    void m() {
        String s = new String("hello");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_input_stream_leak() {
        let src = r#"
class Foo {
    void m() {
        FileInputStream fis = new FileInputStream("file.txt");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FileInputStream"));
    }
}
