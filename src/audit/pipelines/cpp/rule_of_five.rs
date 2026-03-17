use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::cpp_primitives::{
    compile_class_specifier_query, compile_struct_specifier_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct RuleOfFivePipeline {
    class_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl RuleOfFivePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_specifier_query()?,
            struct_query: compile_struct_specifier_query()?,
        })
    }

    fn check_class_body(
        class_name: &str,
        body: tree_sitter::Node,
        class_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
    ) -> Option<AuditFinding> {
        let mut has_destructor = false;
        let mut has_copy_constructor = false;
        let mut has_copy_assignment = false;
        let mut has_move_constructor = false;
        let mut has_move_assignment = false;

        // Walk the field_declaration_list children
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_definition" || child.kind() == "declaration" {
                let text = node_text(child, source);

                // Destructor: ~ClassName
                if text.contains(&format!("~{class_name}")) {
                    has_destructor = true;
                }

                // Copy constructor: ClassName(const ClassName&)
                if text.contains(&format!("{class_name}(const {class_name}"))
                    || text.contains(&format!("{class_name}(const {class_name}&"))
                {
                    has_copy_constructor = true;
                }

                // Copy assignment: operator=(const ClassName&)
                if text.contains("operator=") && text.contains(&format!("const {class_name}")) {
                    has_copy_assignment = true;
                }

                // Move constructor: ClassName(ClassName&&)
                if text.contains(&format!("{class_name}({class_name}&&"))
                    || text.contains(&format!("{class_name}({class_name} &&"))
                {
                    has_move_constructor = true;
                }

                // Move assignment: operator=(ClassName&&)
                if text.contains("operator=") && text.contains(&format!("{class_name}&&")) {
                    has_move_assignment = true;
                }

                // Also check for = default / = delete patterns
                if text.contains(&format!("~{class_name}")) && (text.contains("default") || text.contains("delete")) {
                    has_destructor = true;
                }
            }

            // Also handle access specifiers wrapping declarations
            if child.kind() == "access_specifier" {
                continue;
            }
        }

        if !has_destructor {
            return None;
        }

        let special_count = [
            has_copy_constructor,
            has_copy_assignment,
            has_move_constructor,
            has_move_assignment,
        ]
        .iter()
        .filter(|&&b| b)
        .count();

        // If all four are present, rule of five is satisfied
        if special_count == 4 {
            return None;
        }

        let mut missing = Vec::new();
        if !has_copy_constructor {
            missing.push("copy constructor");
        }
        if !has_copy_assignment {
            missing.push("copy assignment operator");
        }
        if !has_move_constructor {
            missing.push("move constructor");
        }
        if !has_move_assignment {
            missing.push("move assignment operator");
        }

        let start = class_node.start_position();
        Some(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "warning".to_string(),
            pipeline: pipeline_name.to_string(),
            pattern: "missing_rule_of_five".to_string(),
            message: format!(
                "`{class_name}` has a destructor but is missing: {} — violates Rule of Five",
                missing.join(", ")
            ),
            snippet: extract_snippet(source, class_node, 1),
        })
    }
}

impl Pipeline for RuleOfFivePipeline {
    fn name(&self) -> &str {
        "rule_of_five"
    }

    fn description(&self) -> &str {
        "Detects classes with destructor but missing copy/move constructors or assignment operators (Rule of Five)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check classes
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);
            let name_idx = find_capture_index(&self.class_query, "class_name");
            let body_idx = find_capture_index(&self.class_query, "class_body");
            let def_idx = find_capture_index(&self.class_query, "class_def");

            while let Some(m) = matches.next() {
                let name_cap = m.captures.iter().find(|c| c.index as usize == name_idx);
                let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);
                let def_cap = m.captures.iter().find(|c| c.index as usize == def_idx);

                if let (Some(name_cap), Some(body_cap), Some(def_cap)) =
                    (name_cap, body_cap, def_cap)
                {
                    let class_name = node_text(name_cap.node, source);
                    if let Some(finding) = Self::check_class_body(
                        class_name,
                        body_cap.node,
                        def_cap.node,
                        source,
                        file_path,
                        self.name(),
                    ) {
                        findings.push(finding);
                    }
                }
            }
        }

        // Check structs
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.struct_query, tree.root_node(), source);
            let name_idx = find_capture_index(&self.struct_query, "struct_name");
            let body_idx = find_capture_index(&self.struct_query, "struct_body");
            let def_idx = find_capture_index(&self.struct_query, "struct_def");

            while let Some(m) = matches.next() {
                let name_cap = m.captures.iter().find(|c| c.index as usize == name_idx);
                let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);
                let def_cap = m.captures.iter().find(|c| c.index as usize == def_idx);

                if let (Some(name_cap), Some(body_cap), Some(def_cap)) =
                    (name_cap, body_cap, def_cap)
                {
                    let struct_name = node_text(name_cap.node, source);
                    if let Some(finding) = Self::check_class_body(
                        struct_name,
                        body_cap.node,
                        def_cap.node,
                        source,
                        file_path,
                        self.name(),
                    ) {
                        findings.push(finding);
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RuleOfFivePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_missing_rule_of_five() {
        let src = r#"
class Resource {
    int* data;
public:
    ~Resource() { delete data; }
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_rule_of_five");
        assert!(findings[0].message.contains("Resource"));
        assert!(findings[0].message.contains("copy constructor"));
    }

    #[test]
    fn no_finding_for_complete_rule_of_five() {
        let src = r#"
class Resource {
    int* data;
public:
    ~Resource() { delete data; }
    Resource(const Resource& other) : data(new int(*other.data)) {}
    Resource& operator=(const Resource& other) { *data = *other.data; return *this; }
    Resource(Resource&& other) : data(other.data) { other.data = nullptr; }
    Resource& operator=(Resource&& other) { data = other.data; other.data = nullptr; return *this; }
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_without_destructor() {
        let src = r#"
class Simple {
    int x;
public:
    Simple() : x(0) {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_partial_special_members() {
        let src = r#"
class Partial {
    int* data;
public:
    ~Partial() { delete data; }
    Partial(const Partial& other) : data(new int(*other.data)) {}
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("copy assignment"));
        assert!(findings[0].message.contains("move constructor"));
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
class Foo {
public:
    ~Foo() {}
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "rule_of_five");
    }
}
