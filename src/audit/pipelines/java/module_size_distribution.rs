use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, has_modifier};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_top_level_definitions, is_test_file};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_MIN_DEFINITIONS: usize = 1;

const JAVA_TOP_LEVEL_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        // Query to find top-level type declarations with their modifiers
        let exported_query_str = r#"
[
  (class_declaration
    name: (identifier) @name) @def
  (interface_declaration
    name: (identifier) @name) @def
  (enum_declaration
    name: (identifier) @name) @def
  (record_declaration
    name: (identifier) @name) @def
  (annotation_type_declaration
    name: (identifier) @name) @def
]
"#;
        let exported_query = Query::new(&java_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for Java architecture")?;

        Ok(Self {
            exported_query: Arc::new(exported_query),
        })
    }
}

impl Pipeline for ModuleSizeDistributionPipeline {
    fn name(&self) -> &str {
        "module_size_distribution"
    }

    fn description(&self) -> &str {
        "Detects oversized modules, monolithic export surfaces, and anemic modules"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let root = tree.root_node();

        // Count top-level definitions, excluding enum_constant nodes
        let total_definitions = count_top_level_definitions(root, JAVA_TOP_LEVEL_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Find the primary (first) top-level definition kind
        let primary_kind = {
            let mut cursor = root.walk();
            root.children(&mut cursor)
                .find(|c| JAVA_TOP_LEVEL_KINDS.contains(&c.kind()))
                .map(|c| c.kind().to_string())
        };

        // Pattern 1: Oversized module
        // Skip if the primary definition is an enum_declaration or annotation_type_declaration
        let skip_oversized = primary_kind.as_deref() == Some("enum_declaration")
            || primary_kind.as_deref() == Some("annotation_type_declaration");

        if !skip_oversized
            && (total_definitions >= OVERSIZED_SYMBOL_THRESHOLD
                || total_lines >= OVERSIZED_LINE_THRESHOLD)
        {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "oversized_module".to_string(),
                message: format!(
                    "Module has {} definitions and {} lines (thresholds: {} definitions or {} lines)",
                    total_definitions, total_lines, OVERSIZED_SYMBOL_THRESHOLD, OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface
        // Count top-level type declarations that have the `public` modifier
        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let def_idx = find_capture_index(&self.exported_query, "def");
            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == def_idx {
                        // Only count top-level declarations (direct children of program)
                        if cap.node.parent().is_some_and(|p| p.kind() == "program")
                            && has_modifier(cap.node, source, "public")
                        {
                            exported_count += 1;
                        }
                    }
                }
            }
        }

        // Also count public members inside class bodies for monolithic surface
        let mut public_member_count = 0usize;
        {
            let mut walk_cursor = root.walk();
            for child in root.children(&mut walk_cursor) {
                if JAVA_TOP_LEVEL_KINDS.contains(&child.kind())
                    && let Some(body) = child.child_by_field_name("body")
                {
                    let mut body_cursor = body.walk();
                    for member in body.children(&mut body_cursor) {
                        let kind = member.kind();
                        if (kind == "method_declaration"
                            || kind == "field_declaration"
                            || kind == "constructor_declaration"
                            || kind == "class_declaration"
                            || kind == "interface_declaration"
                            || kind == "enum_declaration")
                            && has_modifier(member, source, "public")
                        {
                            public_member_count += 1;
                        }
                    }
                }
            }
        }

        let total_exported = exported_count + public_member_count;
        if total_exported >= MONOLITHIC_EXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "monolithic_export_surface".to_string(),
                message: format!(
                    "Module exports {} symbols (threshold: {})",
                    total_exported, MONOLITHIC_EXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 3: Anemic module
        if total_definitions == ANEMIC_MIN_DEFINITIONS {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| JAVA_TOP_LEVEL_KINDS.contains(&c.kind()))
                    .map(|n| extract_snippet(source, n, 3))
                    .unwrap_or_default()
            };
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "anemic_module".to_string(),
                message:
                    "Module contains only 1 definition — consider merging into a related module"
                        .to_string(),
                snippet,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Foo.java")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&java_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_oversized_module_by_lines() {
        let mut src = String::from("public class Foo {\n");
        for _ in 0..1000 {
            src.push_str("    // filler line\n");
        }
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "public class Foo {\n    void bar() {}\n}\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::from("public class Facade {\n");
        for i in 0..21 {
            src.push_str(&format!("    public void method_{}() {{}}\n", i));
        }
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn no_monolithic_for_private_methods() {
        let mut src = String::from("public class Foo {\n");
        for i in 0..25 {
            src.push_str(&format!("    private void method_{}() {{}}\n", i));
        }
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn detects_anemic_module() {
        let src = "public class Constants {\n    public static final int MAX = 5;\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "class Foo {}\nclass Bar {}\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }
}
