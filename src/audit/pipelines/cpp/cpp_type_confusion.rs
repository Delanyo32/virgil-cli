use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, compile_union_specifier_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct CppTypeConfusionPipeline {
    fn_def_query: Arc<Query>,
    union_query: Arc<Query>,
}

impl CppTypeConfusionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
            union_query: compile_union_specifier_query()?,
        })
    }
}

fn walk_for_casts(
    node: tree_sitter::Node,
    source: &[u8],
    findings: &mut Vec<AuditFinding>,
    file_path: &str,
    pipeline_name: &str,
) {
    let kind = node.kind();

    // In tree-sitter-cpp, C++ named casts (reinterpret_cast, static_cast, etc.)
    // are parsed as call_expression where the function is a template_function.
    // The template_function text starts with the cast keyword.
    if kind == "call_expression" || kind == "template_function" {
        let text = node_text(node, source);

        if text.starts_with("reinterpret_cast") {
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "error".to_string(),
                pipeline: pipeline_name.to_string(),
                pattern: "reinterpret_cast_unrelated".to_string(),
                message:
                    "`reinterpret_cast` between unrelated types — risk of type confusion and undefined behavior"
                        .to_string(),
                snippet: extract_snippet(source, node, 1),
            });
            return;
        }

        if text.starts_with("static_cast") && text.contains('*') {
            // Pointer downcast — check for nearby dynamic_cast guard
            if !has_dynamic_cast_nearby(node, source) {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "error".to_string(),
                    pipeline: pipeline_name.to_string(),
                    pattern: "static_cast_downcast".to_string(),
                    message:
                        "`static_cast` downcast without `dynamic_cast` check — risk of undefined behavior on wrong type"
                            .to_string(),
                    snippet: extract_snippet(source, node, 1),
                });
            }
            return;
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_casts(child, source, findings, file_path, pipeline_name);
    }
}

fn has_dynamic_cast_nearby(node: tree_sitter::Node, source: &[u8]) -> bool {
    // Check if there's a dynamic_cast or typeid check within 3 siblings before this node
    if let Some(parent) = node.parent() {
        let node_idx = {
            let mut idx = 0;
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.id() == node.id() {
                    break;
                }
                idx += 1;
            }
            idx
        };

        let mut cursor = parent.walk();
        for (sibling_idx, child) in parent.children(&mut cursor).enumerate() {
            if sibling_idx >= node_idx {
                break;
            }
            if node_idx - sibling_idx <= 3 {
                let child_text = node_text(child, source);
                if child_text.contains("dynamic_cast") || child_text.contains("typeid") {
                    return true;
                }
            }
        }
    }
    false
}

fn count_typed_fields(union_node: tree_sitter::Node, _source: &[u8]) -> usize {
    // Count field declarations inside the union body
    let mut count = 0;
    let mut cursor = union_node.walk();
    for child in union_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut inner_cursor = child.walk();
            for field in child.children(&mut inner_cursor) {
                if field.kind() == "field_declaration" {
                    count += 1;
                }
            }
        }
    }
    count
}

fn get_field_types(union_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut types = Vec::new();
    let mut cursor = union_node.walk();
    for child in union_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut inner_cursor = child.walk();
            for field in child.children(&mut inner_cursor) {
                if field.kind() == "field_declaration"
                    && let Some(type_node) = field.child_by_field_name("type") {
                        types.push(node_text(type_node, source).to_string());
                    }
            }
        }
    }
    types
}

impl Pipeline for CppTypeConfusionPipeline {
    fn name(&self) -> &str {
        "cpp_type_confusion"
    }

    fn description(&self) -> &str {
        "Detects type confusion: reinterpret_cast between unrelated types, unsafe static_cast downcasts, unchecked variant access"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1 & 2: reinterpret_cast and static_cast via tree walking
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);
            let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

            while let Some(m) = matches.next() {
                let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

                if let Some(body_cap) = body_cap {
                    walk_for_casts(body_cap.node, source, &mut findings, file_path, self.name());
                }
            }
        }

        // Also walk top-level for casts outside functions
        walk_for_casts(
            tree.root_node(),
            source,
            &mut findings,
            file_path,
            self.name(),
        );

        // Deduplicate findings by (line, column, pattern)
        findings.sort_by_key(|f| (f.line, f.column, f.pattern.clone()));
        findings.dedup_by_key(|f| (f.line, f.column, f.pattern.clone()));

        // Pattern 3: union with typed fields (type-punning risk)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.union_query, tree.root_node(), source);
            let union_def_idx = find_capture_index(&self.union_query, "union_def");
            let union_name_idx = find_capture_index(&self.union_query, "union_name");

            while let Some(m) = matches.next() {
                let union_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == union_def_idx);

                if let Some(union_cap) = union_cap {
                    let field_count = count_typed_fields(union_cap.node, source);
                    if field_count <= 1 {
                        continue;
                    }

                    // Check if fields have different types
                    let types = get_field_types(union_cap.node, source);
                    let has_different_types = {
                        let mut unique = types.clone();
                        unique.sort();
                        unique.dedup();
                        unique.len() > 1
                    };

                    if !has_different_types {
                        continue;
                    }

                    let name = m
                        .captures
                        .iter()
                        .find(|c| c.index as usize == union_name_idx)
                        .and_then(|c| c.node.utf8_text(source).ok())
                        .unwrap_or("<anonymous>");

                    let start = union_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "union_inactive_member".to_string(),
                        message: format!(
                            "union `{name}` with typed fields — accessing inactive member is undefined behavior in C++; use `std::variant`"
                        ),
                        snippet: extract_snippet(source, union_cap.node, 1),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppTypeConfusionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_reinterpret_cast() {
        let src = "void f() { int x = 42; float *fp = reinterpret_cast<float*>(&x); }";
        let findings = parse_and_check(src);
        let reinterpret_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "reinterpret_cast_unrelated")
            .collect();
        assert_eq!(reinterpret_findings.len(), 1);
    }

    #[test]
    fn detects_union_typed_fields() {
        let src = "union Data { int i; float f; };";
        let findings = parse_and_check(src);
        let union_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "union_inactive_member")
            .collect();
        assert_eq!(union_findings.len(), 1);
    }

    #[test]
    fn ignores_union_single_field() {
        let src = "union Wrapper { int value; };";
        let findings = parse_and_check(src);
        let union_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "union_inactive_member")
            .collect();
        assert!(union_findings.is_empty());
    }

    #[test]
    fn ignores_union_same_types() {
        let src = "union TwoInts { int a; int b; };";
        let findings = parse_and_check(src);
        let union_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "union_inactive_member")
            .collect();
        assert!(union_findings.is_empty());
    }

    #[test]
    fn detects_static_cast_pointer_downcast() {
        let src = r#"
class Base { virtual ~Base() {} };
class Derived : public Base {};
void f(Base* b) {
    Derived* d = static_cast<Derived*>(b);
}
"#;
        let findings = parse_and_check(src);
        let downcast_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "static_cast_downcast")
            .collect();
        assert_eq!(downcast_findings.len(), 1);
    }

    #[test]
    fn metadata_correct() {
        let src = "void f() { int x = 42; float *fp = reinterpret_cast<float*>(&x); }";
        let findings = parse_and_check(src);
        let f = findings
            .iter()
            .find(|f| f.pattern == "reinterpret_cast_unrelated")
            .unwrap();
        assert_eq!(f.severity, "error");
        assert_eq!(f.pipeline, "cpp_type_confusion");
    }
}
