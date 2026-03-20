use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::find_unreachable_after;

use super::primitives::extract_snippet;

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Walk all `import_statement` nodes at the program root.
    /// Extract imported specifier names. Check if each name appears
    /// elsewhere in the file (outside the import itself).
    fn check_unused_imports(
        root: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "import_statement" {
                continue;
            }

            // For each imported name, check usage outside the import statement
            let specifier_names = Self::extract_import_specifiers(child, source);
            for name in specifier_names {
                // The name must appear somewhere in the file identifiers
                // AND it must appear outside the import statement itself.
                // If the only occurrences are within the import, it's unused.
                if !Self::is_used_outside_import(root, child, source, &name) {
                    let start = child.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: "dead_code".to_string(),
                        pattern: "unused_imports".to_string(),
                        message: format!("imported `{name}` is never used in this file"),
                        snippet: extract_snippet(source, child, 1),
                    });
                }
            }
        }
    }

    /// Extract specifier names from an import statement.
    /// Handles: import { a, b } from '...', import x from '...', import * as y from '...'
    fn extract_import_specifiers(import_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = import_node.walk();

        for child in import_node.children(&mut cursor) {
            match child.kind() {
                // import x from '...'
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        names.push(text.to_string());
                    }
                }
                // import { a, b } from '...'
                "import_clause" => {
                    Self::extract_from_import_clause(child, source, &mut names);
                }
                // import { a, b as c } from '...'  — named_imports inside import_clause
                "named_imports" => {
                    Self::extract_from_named_imports(child, source, &mut names);
                }
                _ => {}
            }
        }

        names
    }

    fn extract_from_import_clause(
        clause: tree_sitter::Node,
        source: &[u8],
        names: &mut Vec<String>,
    ) {
        let mut cursor = clause.walk();
        for child in clause.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        names.push(text.to_string());
                    }
                }
                "named_imports" => {
                    Self::extract_from_named_imports(child, source, names);
                }
                "namespace_import" => {
                    // import * as name
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            names.push(text.to_string());
                        }
                    } else {
                        // fallback: look for identifier child
                        let mut ns_cursor = child.walk();
                        for ns_child in child.children(&mut ns_cursor) {
                            if ns_child.kind() == "identifier"
                                && let Ok(text) = ns_child.utf8_text(source) {
                                    names.push(text.to_string());
                                }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn extract_from_named_imports(
        named: tree_sitter::Node,
        source: &[u8],
        names: &mut Vec<String>,
    ) {
        let mut cursor = named.walk();
        for child in named.named_children(&mut cursor) {
            if child.kind() == "import_specifier" {
                // If there's an alias (import { a as b }), use the alias
                if let Some(alias) = child.child_by_field_name("alias") {
                    if let Ok(text) = alias.utf8_text(source) {
                        names.push(text.to_string());
                    }
                } else if let Some(name_node) = child.child_by_field_name("name")
                    && let Ok(text) = name_node.utf8_text(source) {
                        names.push(text.to_string());
                    }
            }
        }
    }

    /// Check if `name` is used anywhere in the file outside the given import node.
    fn is_used_outside_import(
        root: tree_sitter::Node,
        import_node: tree_sitter::Node,
        source: &[u8],
        name: &str,
    ) -> bool {
        Self::search_usage(root, import_node, source, name)
    }

    fn search_usage(
        root: tree_sitter::Node,
        skip_node: tree_sitter::Node,
        source: &[u8],
        name: &str,
    ) -> bool {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.id() == skip_node.id() {
                continue;
            }

            let kind = node.kind();
            if (kind == "identifier"
                || kind == "property_identifier"
                || kind == "shorthand_property_identifier")
                && node.utf8_text(source).unwrap_or("") == name
            {
                return true;
            }

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        false
    }

    /// Walk all `statement_block` nodes. After any return/break/continue/throw,
    /// flag subsequent statements as unreachable.
    fn check_unreachable_code(
        root: tree_sitter::Node,
        _source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "statement_block" {
                let return_kinds = [
                    "return_statement",
                    "break_statement",
                    "continue_statement",
                    "throw_statement",
                ];
                let unreachable = find_unreachable_after(node, &return_kinds);
                for (line, column) in unreachable {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line,
                        column,
                        severity: "warning".to_string(),
                        pipeline: "dead_code".to_string(),
                        pattern: "unreachable_code".to_string(),
                        message: "code after return/break/continue/throw is unreachable"
                            .to_string(),
                        snippet: String::new(),
                    });
                }
            }

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused imports and unreachable code in JavaScript files"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        Self::check_unused_imports(root, source, file_path, &mut findings);
        Self::check_unreachable_code(root, source, file_path, &mut findings);

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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    // ── unused_imports ─────────────────────────────────────────────

    #[test]
    fn detects_unused_named_import() {
        let src = r#"import { foo } from './bar';
console.log("hello");
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("foo"));
    }

    #[test]
    fn skips_used_import() {
        let src = r#"import { foo } from './bar';
foo();
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn detects_unused_default_import() {
        let src = r#"import React from 'react';
console.log("no react here");
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("React"));
    }

    // ── unreachable_code ───────────────────────────────────────────

    #[test]
    fn detects_unreachable_after_return() {
        let src = r#"function foo() {
    return 1;
    console.log("dead");
}"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert_eq!(unreachable.len(), 1);
        assert_eq!(unreachable[0].line, 3);
    }

    #[test]
    fn no_unreachable_in_clean_function() {
        let src = r#"function foo() {
    const x = 1;
    return x;
}"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(unreachable.is_empty());
    }

    #[test]
    fn detects_unreachable_after_throw() {
        let src = r#"function bar() {
    throw new Error("fail");
    doSomething();
}"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert_eq!(unreachable.len(), 1);
    }

    // ── metadata ───────────────────────────────────────────────────

    #[test]
    fn pipeline_metadata() {
        let pipeline = DeadCodePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "dead_code");
        assert!(!pipeline.description().is_empty());
    }
}
