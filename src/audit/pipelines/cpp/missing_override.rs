use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_class_specifier_query, compile_struct_specifier_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct MissingOverridePipeline {
    class_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl MissingOverridePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_specifier_query()?,
            struct_query: compile_struct_specifier_query()?,
        })
    }

    fn has_base_class(class_node: tree_sitter::Node) -> bool {
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "base_class_clause" {
                return true;
            }
        }
        false
    }

    fn has_virtual_specifier(node: tree_sitter::Node) -> bool {
        // Check for virtual_function_specifier child node (the `virtual` keyword)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "virtual_function_specifier" || child.kind() == "virtual" {
                return true;
            }
        }
        false
    }

    fn has_override_or_final(node: tree_sitter::Node) -> bool {
        // Check for virtual_specifier child (override/final) in the declarator tree
        fn find_specifier(n: tree_sitter::Node) -> bool {
            if n.kind() == "virtual_specifier" {
                return true;
            }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if find_specifier(child) {
                    return true;
                }
            }
            false
        }
        find_specifier(node)
    }

    fn is_destructor(node: tree_sitter::Node) -> bool {
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

    fn has_pure_virtual_clause(node: tree_sitter::Node) -> bool {
        fn find_pure_virtual(n: tree_sitter::Node) -> bool {
            if n.kind() == "pure_virtual_clause" {
                return true;
            }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if find_pure_virtual(child) {
                    return true;
                }
            }
            false
        }
        find_pure_virtual(node)
    }

    fn check_body_for_missing_override(
        body: tree_sitter::Node,
        class_name: &str,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            let is_func = child.kind() == "function_definition" || child.kind() == "declaration";
            if !is_func {
                continue;
            }

            // Must have virtual keyword (via AST node kind, not string matching)
            if !Self::has_virtual_specifier(child) {
                continue;
            }

            // Skip destructors (via AST node kind)
            if Self::is_destructor(child) {
                continue;
            }

            // Skip if override or final is present (via AST node kind)
            if Self::has_override_or_final(child) {
                continue;
            }

            // Skip pure virtual (= 0) declarations
            if Self::has_pure_virtual_clause(child) {
                continue;
            }

            if is_nolint_suppressed(source, child, pipeline_name) {
                continue;
            }

            let start = child.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
                pipeline: pipeline_name.to_string(),
                pattern: "missing_override".to_string(),
                message: format!(
                    "virtual method in `{class_name}` without `override` specifier — add `override` to ensure it matches a base class method"
                ),
                snippet: extract_snippet(source, child, 1),
            });
        }

        findings
    }
}

impl GraphPipeline for MissingOverridePipeline {
    fn name(&self) -> &str {
        "missing_override"
    }

    fn description(&self) -> &str {
        "Detects virtual methods in derived classes without the override specifier"
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
                    if !Self::has_base_class(def_cap.node) {
                        continue;
                    }

                    let class_name = node_text(name_cap.node, source);
                    findings.extend(Self::check_body_for_missing_override(
                        body_cap.node,
                        class_name,
                        source,
                        file_path,
                        self.name(),
                    ));
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
                    if !Self::has_base_class(def_cap.node) {
                        continue;
                    }

                    let struct_name = node_text(name_cap.node, source);
                    findings.extend(Self::check_body_for_missing_override(
                        body_cap.node,
                        struct_name,
                        source,
                        file_path,
                        self.name(),
                    ));
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
        let pipeline = MissingOverridePipeline::new().unwrap();
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
    fn detects_missing_override() {
        let src = r#"
class Base {
    virtual void foo() {}
};
class Derived : public Base {
    virtual void foo() {}
};
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].pattern, "missing_override");
        assert!(findings[0].message.contains("Derived"));
    }

    #[test]
    fn no_finding_with_override() {
        let src = r#"
class Base {
    virtual void foo() {}
};
class Derived : public Base {
    void foo() override {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_without_base_class() {
        let src = r#"
class Base {
    virtual void foo() {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_pure_virtual() {
        let src = r#"
class Base {
    virtual void foo() = 0;
};
class Derived : public Base {
    virtual void bar() = 0;
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_with_final() {
        let src = r#"
class Base {
    virtual void foo() {}
};
class Derived : public Base {
    void foo() final {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
class Base { virtual void f() {} };
class D : public Base { virtual void f() {} };
"#;
        let findings = parse_and_check(src);
        if !findings.is_empty() {
            assert_eq!(findings[0].severity, "warning");
            assert_eq!(findings[0].pipeline, "missing_override");
        }
    }

    #[test]
    fn nolint_suppression() {
        let src = r#"
class Base { virtual void f() {} };
class D : public Base {
    virtual void f() {} // NOLINT
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
