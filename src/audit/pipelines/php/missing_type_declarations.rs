use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_def_query, compile_method_decl_query, find_capture_index, node_text,
};

const MAGIC_METHODS: &[&str] = &[
    "__construct",
    "__destruct",
    "__toString",
    "__clone",
    "__debugInfo",
    "__invoke",
    "__sleep",
    "__wakeup",
    "__serialize",
    "__unserialize",
    "__set_state",
];

pub struct MissingTypeDeclarationsPipeline {
    fn_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl MissingTypeDeclarationsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
            method_query: compile_method_decl_query()?,
        })
    }
}

impl Pipeline for MissingTypeDeclarationsPipeline {
    fn name(&self) -> &str {
        "missing_type_declarations"
    }

    fn description(&self) -> &str {
        "Detects functions and methods missing parameter or return type declarations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_functions(tree, source, file_path, &mut findings);
        self.check_methods(tree, source, file_path, &mut findings);
        findings
    }
}

impl MissingTypeDeclarationsPipeline {
    fn check_functions(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let params_idx = find_capture_index(&self.fn_query, "params");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(params_node), Some(def_node)) =
                (name_node, params_node, def_node)
            {
                let fn_name = node_text(name_node, source);
                let start = def_node.start_position();

                check_return_type(
                    def_node,
                    fn_name,
                    file_path,
                    start,
                    "missing_type_declarations",
                    findings,
                );
                check_param_types(
                    params_node,
                    fn_name,
                    file_path,
                    start,
                    "missing_type_declarations",
                    source,
                    findings,
                );
            }
        }
    }

    fn check_methods(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.method_query, "method_name");
        let params_idx = find_capture_index(&self.method_query, "params");
        let method_decl_idx = find_capture_index(&self.method_query, "method_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(params_node), Some(decl_node)) =
                (name_node, params_node, decl_node)
            {
                let method_name = node_text(name_node, source);

                // Skip magic methods
                if MAGIC_METHODS.contains(&method_name) {
                    continue;
                }

                let start = decl_node.start_position();

                check_return_type(
                    decl_node,
                    method_name,
                    file_path,
                    start,
                    "missing_type_declarations",
                    findings,
                );
                check_param_types(
                    params_node,
                    method_name,
                    file_path,
                    start,
                    "missing_type_declarations",
                    source,
                    findings,
                );
            }
        }
    }
}

fn check_return_type(
    def_node: tree_sitter::Node,
    name: &str,
    file_path: &str,
    start: tree_sitter::Point,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    // In PHP tree-sitter, return type is in a `return_type` field
    let has_return_type = def_node.child_by_field_name("return_type").is_some();
    if !has_return_type {
        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "info".to_string(),
            pipeline: pipeline_name.to_string(),
            pattern: "missing_return_type".to_string(),
            message: format!("`{name}` is missing a return type declaration"),
            snippet: format!("function {name}(...)"),
        });
    }
}

fn check_param_types(
    params_node: tree_sitter::Node,
    name: &str,
    file_path: &str,
    start: tree_sitter::Point,
    pipeline_name: &str,
    source: &[u8],
    findings: &mut Vec<AuditFinding>,
) {
    let untyped_params: Vec<String> = (0..params_node.named_child_count())
        .filter_map(|i| params_node.named_child(i))
        .filter(|child| child.kind() == "simple_parameter")
        .filter(|child| child.child_by_field_name("type").is_none())
        .filter_map(|child| {
            child
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
        })
        .collect();

    if !untyped_params.is_empty() {
        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "info".to_string(),
            pipeline: pipeline_name.to_string(),
            pattern: "missing_param_type".to_string(),
            message: format!(
                "`{name}` has untyped parameters: {}",
                untyped_params.join(", ")
            ),
            snippet: format!("function {name}(...)"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTypeDeclarationsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_missing_return_and_param_types() {
        let src = "<?php\nfunction foo($x, $y) { return $x + $y; }\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "missing_return_type"));
        assert!(findings.iter().any(|f| f.pattern == "missing_param_type"));
    }

    #[test]
    fn clean_fully_typed_function() {
        let src = "<?php\nfunction foo(int $x, string $y): bool { return true; }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_missing_method_types() {
        let src = "<?php\nclass Foo {\n    public function bar($x) { }\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "missing_return_type"));
        assert!(findings.iter().any(|f| f.pattern == "missing_param_type"));
    }

    #[test]
    fn skips_magic_methods() {
        let src = "<?php\nclass Foo {\n    public function __construct($x) { }\n    public function __toString() { return ''; }\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_missing_return_only() {
        let src = "<?php\nfunction foo(int $x) { return $x; }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_return_type");
    }

    #[test]
    fn clean_fully_typed_method() {
        let src = "<?php\nclass Foo {\n    public function bar(int $x): void { }\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
