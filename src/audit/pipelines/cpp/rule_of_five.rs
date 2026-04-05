use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_class_specifier_query, compile_struct_specifier_query, extract_snippet,
    find_capture_index, find_identifier_in_declarator, node_text,
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

    fn has_virtual_specifier(node: tree_sitter::Node, _source: &[u8]) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "virtual_function_specifier"
                || child.kind() == "virtual"
            {
                return true;
            }
        }
        false
    }

    fn is_destructor(node: tree_sitter::Node, _source: &[u8]) -> bool {
        // Check for destructor_name in the declarator tree
        fn find_destructor_name(n: tree_sitter::Node) -> bool {
            if n.kind() == "destructor_name" {
                return true;
            }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if find_destructor_name(child) {
                    return true;
                }
            }
            false
        }
        find_destructor_name(node)
    }

    fn is_defaulted_or_deleted(node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(node, source);
        // Look for = default or = delete at the end of declaration
        text.ends_with("= default;") || text.ends_with("= delete;")
            || text.contains("= default") || text.contains("= delete")
    }

    fn is_copy_constructor(node: tree_sitter::Node, class_name: &str, source: &[u8]) -> bool {
        // Get function name from declarator
        if let Some(declarator) = node.child_by_field_name("declarator") {
            if let Some(name) = find_identifier_in_declarator(declarator, source) {
                if name != class_name {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            return false;
        }

        // Check parameters for const ClassName&
        let text = node_text(node, source);
        text.contains(&format!("const {class_name}&"))
            || text.contains(&format!("const {class_name} &"))
    }

    fn is_move_constructor(node: tree_sitter::Node, class_name: &str, source: &[u8]) -> bool {
        if let Some(declarator) = node.child_by_field_name("declarator") {
            if let Some(name) = find_identifier_in_declarator(declarator, source) {
                if name != class_name {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            return false;
        }

        let text = node_text(node, source);
        (text.contains(&format!("{class_name}&&")) || text.contains(&format!("{class_name} &&")))
            && !text.contains("const")
    }

    fn is_copy_assignment(node: tree_sitter::Node, class_name: &str, source: &[u8]) -> bool {
        // Check for operator= with const ref parameter
        fn has_operator_name(n: tree_sitter::Node) -> bool {
            if n.kind() == "operator_name" {
                return true;
            }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if has_operator_name(child) {
                    return true;
                }
            }
            false
        }

        if let Some(declarator) = node.child_by_field_name("declarator") {
            if !has_operator_name(declarator) {
                return false;
            }
        } else {
            return false;
        }

        let text = node_text(node, source);
        text.contains("operator=")
            && (text.contains(&format!("const {class_name}&"))
                || text.contains(&format!("const {class_name} &")))
    }

    fn is_move_assignment(node: tree_sitter::Node, class_name: &str, source: &[u8]) -> bool {
        fn has_operator_name(n: tree_sitter::Node) -> bool {
            if n.kind() == "operator_name" {
                return true;
            }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if has_operator_name(child) {
                    return true;
                }
            }
            false
        }

        if let Some(declarator) = node.child_by_field_name("declarator") {
            if !has_operator_name(declarator) {
                return false;
            }
        } else {
            return false;
        }

        let text = node_text(node, source);
        text.contains("operator=")
            && (text.contains(&format!("{class_name}&&"))
                || text.contains(&format!("{class_name} &&")))
            && !text.contains(&format!("const {class_name}"))
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
        let mut destructor_is_virtual_default = false;

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            let is_func = child.kind() == "function_definition" || child.kind() == "declaration";
            if !is_func {
                continue;
            }

            // Destructor detection via AST
            if Self::is_destructor(child, source) {
                has_destructor = true;
                // Check for virtual ~Foo() = default (polymorphic, no resources)
                if Self::has_virtual_specifier(child, source)
                    && Self::is_defaulted_or_deleted(child, source)
                {
                    destructor_is_virtual_default = true;
                }
                continue;
            }

            // Check for = default or = delete on special members (count them as defined)
            let is_defaulted_or_deleted = Self::is_defaulted_or_deleted(child, source);

            if Self::is_copy_constructor(child, class_name, source) || (is_defaulted_or_deleted && {
                let text = node_text(child, source);
                text.contains(class_name) && text.contains("const") && !text.contains("operator")
            }) {
                has_copy_constructor = true;
            }

            if Self::is_copy_assignment(child, class_name, source) {
                has_copy_assignment = true;
            }

            if Self::is_move_constructor(child, class_name, source) {
                has_move_constructor = true;
            }

            if Self::is_move_assignment(child, class_name, source) {
                has_move_assignment = true;
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

        if special_count == 4 {
            return None;
        }

        // NOLINT check
        if is_nolint_suppressed(source, class_node, pipeline_name) {
            return None;
        }

        // Virtual default destructor (polymorphic but no resources) gets lower severity
        let severity = if destructor_is_virtual_default {
            "info"
        } else {
            "warning"
        };

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
            severity: severity.to_string(),
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

impl GraphPipeline for RuleOfFivePipeline {
    fn name(&self) -> &str {
        "rule_of_five"
    }

    fn description(&self) -> &str {
        "Detects classes with destructor but missing copy/move constructors or assignment operators (Rule of Five)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RuleOfFivePipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.cpp",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn nolint_suppression() {
        let src = r#"
class Foo { // NOLINT
public:
    ~Foo() {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn virtual_default_destructor_info() {
        let src = r#"
class Base {
public:
    virtual ~Base() = default;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }
}
