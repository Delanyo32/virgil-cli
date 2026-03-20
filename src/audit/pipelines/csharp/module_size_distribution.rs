use anyhow::Result;
use tree_sitter::Tree;

use super::primitives::{extract_snippet, has_modifier};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::is_entry_file;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_ENTRY_FILES: &[&str] = &["Program.cs"];

/// Type declarations used for counting definitions. C# typically nests
/// these inside namespace bodies, so we walk into them.
const CSHARP_TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "interface_declaration",
    "enum_declaration",
    "delegate_declaration",
];

/// Count all type declarations in the file, walking into namespace bodies.
fn count_all_type_definitions(node: tree_sitter::Node) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if CSHARP_TYPE_KINDS.contains(&child.kind()) {
            count += 1;
        }
        if child.kind() == "namespace_declaration"
            && let Some(body) = child.child_by_field_name("body") {
                count += count_all_type_definitions(body);
            }
    }
    count
}

/// Count exported type declarations (public or internal modifier), walking into namespace bodies.
fn count_exported_type_definitions(node: tree_sitter::Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if CSHARP_TYPE_KINDS.contains(&child.kind())
            && (has_modifier(child, source, "public") || has_modifier(child, source, "internal")) {
                count += 1;
            }
        if child.kind() == "namespace_declaration"
            && let Some(body) = child.child_by_field_name("body") {
                count += count_exported_type_definitions(body, source);
            }
    }
    count
}

pub struct ModuleSizeDistributionPipeline;

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
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
        let mut findings = Vec::new();
        let root = tree.root_node();

        let total_definitions = count_all_type_definitions(root);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module
        if total_definitions >= OVERSIZED_SYMBOL_THRESHOLD
            || total_lines >= OVERSIZED_LINE_THRESHOLD
        {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "oversized_module".to_string(),
                message: format!(
                    "Module has {} type definitions and {} lines (thresholds: {} definitions or {} lines)",
                    total_definitions, total_lines, OVERSIZED_SYMBOL_THRESHOLD, OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface
        let exported_count = count_exported_type_definitions(root, source);

        if exported_count >= MONOLITHIC_EXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "monolithic_export_surface".to_string(),
                message: format!(
                    "Module exports {} type symbols (threshold: {})",
                    exported_count, MONOLITHIC_EXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 3: Anemic module
        if total_definitions == 1 && !is_entry_file(file_path, ANEMIC_ENTRY_FILES) {
            let snippet = find_first_type_snippet(root, source);
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "anemic_module".to_string(),
                message: "Module contains only 1 type definition \u{2014} consider merging into a related module".to_string(),
                snippet,
            });
        }

        findings
    }
}

/// Find the first type declaration and extract a snippet from it.
fn find_first_type_snippet(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if CSHARP_TYPE_KINDS.contains(&child.kind()) {
            return extract_snippet(source, child, 3);
        }
        if child.kind() == "namespace_declaration"
            && let Some(body) = child.child_by_field_name("body") {
                let inner = find_first_type_snippet(body, source);
                if !inner.is_empty() {
                    return inner;
                }
            }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn csharp_lang() -> tree_sitter::Language {
        Language::CSharp.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::from("namespace MyApp {\n");
        for i in 0..31 {
            src.push_str(&format!("public class Class_{} {{ }}\n", i));
        }
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = r#"
namespace MyApp {
    public class Foo { }
    public class Bar { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::from("namespace MyApp {\n");
        for i in 0..21 {
            src.push_str(&format!("public class Svc_{} {{ }}\n", i));
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
    fn detects_anemic_module() {
        let src = r#"
namespace MyApp {
    public class Constants { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&csharp_lang()).unwrap();
        let src = r#"
namespace MyApp {
    public class Program { }
}
"#;
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "Program.cs");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = r#"
namespace MyApp {
    public class Foo { }
    public class Bar { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }
}
