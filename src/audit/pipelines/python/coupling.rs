use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{Pipeline, PipelineContext};
use crate::audit::pipelines::helpers::{body_has_member_access, count_nodes_of_kind};

use super::primitives::{extract_snippet, node_text};

const EXCESSIVE_IMPORTS_THRESHOLD: usize = 15;
const PARAMETER_OVERLOAD_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_excessive_imports(
        &self,
        tree: &Tree,
        _source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let import_count =
            count_nodes_of_kind(root, &["import_statement", "import_from_statement"]);

        if import_count > EXCESSIVE_IMPORTS_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_imports".to_string(),
                message: format!(
                    "file has {import_count} imports (threshold: {EXCESSIVE_IMPORTS_THRESHOLD}) \
                     — consider splitting into smaller modules"
                ),
                snippet: String::new(),
            });
        }

        findings
    }

    fn check_parameter_overload(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        collect_parameter_overloads(root, source, file_path, self.name(), &mut findings);

        findings
    }

    fn check_low_cohesion(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        collect_low_cohesion_methods(root, source, file_path, self.name(), &mut findings);

        findings
    }

    /// Find a `function_definition` node whose start row matches the given line (0-indexed).
    fn find_function_at_line<'a>(
        &self,
        root: tree_sitter::Node<'a>,
        line: u32,
    ) -> Option<tree_sitter::Node<'a>> {
        let target_row = line as usize;
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "function_definition" && node.start_position().row == target_row {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        None
    }

    /// Walk up parents from a node to find the enclosing `class_definition`.
    fn find_enclosing_class<'a>(
        &self,
        node: tree_sitter::Node<'a>,
    ) -> Option<tree_sitter::Node<'a>> {
        let mut current = node.parent();
        while let Some(n) = current {
            if n.kind() == "class_definition" {
                return Some(n);
            }
            current = n.parent();
        }
        None
    }

    /// Check if a class_definition inherits from ABC, Protocol, or similar base classes.
    fn class_has_abc_base(&self, class_node: tree_sitter::Node, source: &[u8]) -> bool {
        const ABC_BASES: &[&str] = &[
            "ABC",
            "ABCMeta",
            "Protocol",
            "Interface",
            "BaseModel",
            "Base",
        ];

        // The superclasses are in the argument_list child of class_definition
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "argument_list" {
                let mut arg_cursor = child.walk();
                for arg in child.children(&mut arg_cursor) {
                    match arg.kind() {
                        "identifier" => {
                            let name = node_text(arg, source);
                            if ABC_BASES.contains(&name) {
                                return true;
                            }
                        }
                        // Handle `metaclass=ABCMeta` keyword argument
                        "keyword_argument" => {
                            if let Some(value) = arg.child_by_field_name("value") {
                                let val_text = node_text(value, source);
                                if val_text == "ABCMeta" {
                                    return true;
                                }
                            }
                        }
                        // Handle dotted names like `abc.ABC`
                        "attribute" => {
                            let attr_text = node_text(arg, source);
                            for base in ABC_BASES {
                                if attr_text.ends_with(&format!(".{base}")) {
                                    return true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    /// Check if a method has a decorator that should suppress low_cohesion findings.
    fn method_has_suppressing_decorator(
        &self,
        func_node: tree_sitter::Node,
        source: &[u8],
    ) -> bool {
        const SUPPRESSING_DECORATORS: &[&str] = &[
            "abstractmethod",
            "property",
            "override",
            "validator",
            "field_validator",
            "classmethod",
            "staticmethod",
        ];

        // The function_definition's parent may be a decorated_definition
        let parent = match func_node.parent() {
            Some(p) if p.kind() == "decorated_definition" => p,
            _ => return false,
        };

        let mut cursor = parent.walk();
        for child in parent.children(&mut cursor) {
            if child.kind() == "decorator" {
                // The decorator's child is the expression after @
                // It can be an identifier (@abstractmethod) or an attribute (@abc.abstractmethod)
                // or a call (@validator("field"))
                if let Some(expr) = child.named_child(0) {
                    let decorator_name = match expr.kind() {
                        "identifier" => node_text(expr, source),
                        "attribute" => {
                            // e.g., abc.abstractmethod — get the last segment
                            if let Some(attr) = expr.child_by_field_name("attribute") {
                                node_text(attr, source)
                            } else {
                                continue;
                            }
                        }
                        "call" => {
                            // e.g., @validator("field") — get the function name
                            if let Some(func) = expr.child_by_field_name("function") {
                                node_text(func, source)
                            } else {
                                continue;
                            }
                        }
                        _ => continue,
                    };
                    if SUPPRESSING_DECORATORS.contains(&decorator_name) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns true if the finding's method is part of an interface/abstract pattern
    /// and should be suppressed.
    fn is_interface_method(
        &self,
        finding: &AuditFinding,
        tree: &Tree,
        source: &[u8],
    ) -> bool {
        let root = tree.root_node();
        // Finding line is 1-indexed, tree-sitter rows are 0-indexed
        let target_row = finding.line.saturating_sub(1);

        let func_node = match self.find_function_at_line(root, target_row) {
            Some(n) => n,
            None => return false,
        };

        // Check if the method has a suppressing decorator
        if self.method_has_suppressing_decorator(func_node, source) {
            return true;
        }

        // Check if the enclosing class inherits from ABC/Protocol
        if let Some(class_node) = self.find_enclosing_class(func_node) {
            if self.class_has_abc_base(class_node, source) {
                return true;
            }
        }

        false
    }
}

/// Count parameters in a Python parameters node, excluding `self` and `cls`.
fn count_python_parameters(params_node: tree_sitter::Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        let kind = child.kind();
        // Skip variadic parameters (*args, **kwargs)
        if kind == "list_splat_pattern" || kind == "dictionary_splat_pattern" {
            continue;
        }
        // For identifiers (untyped params), check if it's self/cls
        if kind == "identifier" {
            let name = node_text(child, source);
            if name == "self" || name == "cls" {
                continue;
            }
        }
        // For typed_parameter, check the name
        if kind == "typed_parameter"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let name = node_text(name_node, source);
            if name == "self" || name == "cls" {
                continue;
            }
        }
        // For default_parameter, check the name
        if kind == "default_parameter"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let name = node_text(name_node, source);
            if name == "self" || name == "cls" {
                continue;
            }
        }
        // For typed_default_parameter, check the name
        if kind == "typed_default_parameter"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let name = node_text(name_node, source);
            if name == "self" || name == "cls" {
                continue;
            }
        }
        count += 1;
    }
    count
}

/// Check if a function has a `self` parameter.
fn has_self_parameter(params_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if node_text(child, source) == "self" {
                    return true;
                }
            }
            "typed_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name")
                    && node_text(name_node, source) == "self"
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Walk tree looking for function_definition nodes and check parameter counts.
fn collect_parameter_overloads(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition"
            && let Some(name_node) = node.child_by_field_name("name")
        {
            let fn_name = node_text(name_node, source);
            // Skip __init__ constructors — they often need many parameters
            if fn_name != "__init__"
                && let Some(params_node) = node.child_by_field_name("parameters")
            {
                let param_count = count_python_parameters(params_node, source);
                if param_count > PARAMETER_OVERLOAD_THRESHOLD {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "parameter_overload".to_string(),
                        message: format!(
                            "function `{fn_name}` has {param_count} parameters \
                                     (threshold: {PARAMETER_OVERLOAD_THRESHOLD}) — consider using \
                                     a configuration object or dataclass"
                        ),
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Walk tree looking for methods inside class_definition that have a `self` parameter
/// but don't access `self` in their body.
fn collect_low_cohesion_methods(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "class_definition" {
            // Walk the class body looking for methods
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    let func_node = match child.kind() {
                        "function_definition" => child,
                        "decorated_definition" => match find_inner_function(child) {
                            Some(f) => f,
                            None => continue,
                        },
                        _ => continue,
                    };

                    let name_node = match func_node.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let fn_name = node_text(name_node, source);

                    // Skip dunder methods
                    if fn_name.starts_with("__") && fn_name.ends_with("__") {
                        continue;
                    }

                    let params_node = match func_node.child_by_field_name("parameters") {
                        Some(p) => p,
                        None => continue,
                    };

                    // Only check methods that have a `self` parameter
                    if !has_self_parameter(params_node, source) {
                        continue;
                    }

                    let func_body = match func_node.child_by_field_name("body") {
                        Some(b) => b,
                        None => continue,
                    };

                    // Skip trivial bodies (just `pass` or `...`)
                    if is_trivial_body(func_body, source) {
                        continue;
                    }

                    // Check if the body accesses self via attribute access
                    if !body_has_member_access(func_body, source, "attribute", "self") {
                        let start = func_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "low_cohesion".to_string(),
                            message: format!(
                                "method `{fn_name}` has a `self` parameter but never accesses \
                                 instance attributes — consider making it a function or @staticmethod"
                            ),
                            snippet: extract_snippet(source, func_node, 1),
                        });
                    }
                }
            }

            // Don't recurse into nested classes from here; we'll catch them naturally
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Find the inner function_definition inside a decorated_definition.
fn find_inner_function(decorated: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = decorated.walk();
    decorated
        .children(&mut cursor)
        .find(|&child| child.kind() == "function_definition")
}

/// Check if a function body is trivial (just `pass` or `...`).
fn is_trivial_body(body: tree_sitter::Node, _source: &[u8]) -> bool {
    if body.named_child_count() != 1 {
        return false;
    }
    if let Some(child) = body.named_child(0) {
        let kind = child.kind();
        if kind == "pass_statement" {
            return true;
        }
        if kind == "expression_statement"
            && let Some(expr) = child.named_child(0)
            && expr.kind() == "ellipsis"
        {
            return true;
        }
    }
    false
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low cohesion in Python"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_excessive_imports(tree, source, file_path));
        findings.extend(self.check_parameter_overload(tree, source, file_path));
        findings.extend(self.check_low_cohesion(tree, source, file_path));
        findings
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        let base = self.check(ctx.tree, ctx.source, ctx.file_path);

        base.into_iter()
            .filter(|f| {
                if f.pattern != "low_cohesion" {
                    return true; // pass through non-cohesion findings
                }
                !self.is_interface_method(f, ctx.tree, ctx.source)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let imports: String = (0..16).map(|i| format!("import mod{i}\n")).collect();
        let src = format!("{imports}\ndef main():\n    pass\n");
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn clean_few_imports() {
        let src = "\
import os
import sys

def main():
    pass
";
        let findings = parse_and_check(src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(excessive.is_empty());
    }

    // ── parameter_overload ──

    #[test]
    fn detects_parameter_overload() {
        let src = "\
def process(a, b, c, d, e, f):
    pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
        assert!(overloads[0].message.contains("process"));
    }

    #[test]
    fn clean_few_parameters() {
        let src = "\
def process(a, b, c):
    pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloads.is_empty());
    }

    #[test]
    fn excludes_self_from_count() {
        let src = "\
class Foo:
    def process(self, a, b, c, d, e):
        pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloads.is_empty());
    }

    #[test]
    fn detects_overload_even_with_self() {
        let src = "\
class Foo:
    def process(self, a, b, c, d, e, f):
        pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
    }

    // ── low_cohesion ──

    #[test]
    fn detects_low_cohesion() {
        let src = "\
class Calculator:
    def add(self, a, b):
        return a + b
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("add"));
    }

    #[test]
    fn clean_uses_self() {
        let src = "\
class Calculator:
    def add(self, a, b):
        self.result = a + b
        return self.result
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_trivial_methods() {
        let src = "\
class Base:
    def do_nothing(self):
        pass
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_static_methods() {
        // No self parameter, so not flagged
        let src = "\
class Util:
    def helper(a, b):
        return a + b
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_dunder_methods() {
        let src = "\
class Foo:
    def __repr__(self):
        return 'Foo()'
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    // ── check_with_context low_cohesion filtering ──

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        let id_counts = std::collections::HashMap::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: None,
        };
        pipeline.check_with_context(&ctx)
    }

    #[test]
    fn context_suppresses_abc_method() {
        let src = "\
from abc import ABC

class Handler(ABC):
    def handle(self, data):
        return data
";
        let findings = parse_and_check_with_context(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty(), "ABC methods should not be flagged");
    }

    #[test]
    fn context_suppresses_protocol_method() {
        let src = "\
from typing import Protocol

class Processor(Protocol):
    def process(self, item):
        return item
";
        let findings = parse_and_check_with_context(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty(), "Protocol methods should not be flagged");
    }

    #[test]
    fn context_suppresses_abstractmethod() {
        let src = "\
from abc import ABC, abstractmethod

class Base(ABC):
    @abstractmethod
    def execute(self, cmd):
        raise NotImplementedError
";
        let findings = parse_and_check_with_context(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(
            low.is_empty(),
            "@abstractmethod methods should not be flagged"
        );
    }

    #[test]
    fn context_suppresses_property_decorator() {
        let src = "\
class Config:
    @property
    def name(self):
        return \"test\"
";
        let findings = parse_and_check_with_context(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty(), "@property methods should not be flagged");
    }

    #[test]
    fn context_still_flags_regular_low_cohesion() {
        let src = "\
class Calculator:
    def add(self, a, b):
        return a + b
";
        let findings = parse_and_check_with_context(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(
            low.len(),
            1,
            "Regular methods not using self should still be flagged"
        );
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = CouplingPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "coupling");
        assert!(!pipeline.description().is_empty());
    }
}
