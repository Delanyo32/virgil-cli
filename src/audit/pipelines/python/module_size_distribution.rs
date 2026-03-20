use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_top_level_definitions, is_entry_file, is_test_file};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_DEFINITION_THRESHOLD: usize = 1;
const ANEMIC_ENTRY_FILES: &[&str] = &[
    "__init__.py",
    "__main__.py",
    "conftest.py",
    "setup.py",
    "config.py",
    "settings.py",
    "constants.py",
    "urls.py",
    "admin.py",
];

const PYTHON_DEFINITION_KINDS: &[&str] = &[
    "function_definition",
    "class_definition",
    "decorated_definition",
];

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        // Match top-level function/class definitions whose name does NOT start with _
        // For Python, "exported" means the name does not start with underscore.
        // We capture the name to check at runtime since tree-sitter predicates are not
        // universally supported in the Rust bindings.
        let exported_query_str = r#"
[
  (function_definition
    name: (identifier) @name) @def
  (class_definition
    name: (identifier) @name) @def
  (decorated_definition
    definition: (function_definition
      name: (identifier) @name)) @def
  (decorated_definition
    definition: (class_definition
      name: (identifier) @name)) @def
]
"#;
        let exported_query = Query::new(&python_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for Python architecture")?;

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
        // Skip .pyi type stub files entirely
        if file_path.ends_with(".pyi") {
            return Vec::new();
        }

        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let root = tree.root_node();

        let total_definitions = count_top_level_definitions(root, PYTHON_DEFINITION_KINDS);
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
                    "Module has {} definitions and {} lines (thresholds: {} definitions or {} lines)",
                    total_definitions, total_lines, OVERSIZED_SYMBOL_THRESHOLD, OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface
        // Count top-level symbols whose name does NOT start with underscore
        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let def_idx = find_capture_index(&self.exported_query, "def");
            let name_idx = find_capture_index(&self.exported_query, "name");
            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                let mut is_top_level = false;
                let mut name_starts_with_underscore = true;

                for cap in m.captures {
                    if cap.index as usize == def_idx {
                        // Check if this is a direct child of the module (top-level)
                        is_top_level = cap.node.parent().is_some_and(|p| p.kind() == "module");
                    }
                    if cap.index as usize == name_idx {
                        let name = node_text(cap.node, source);
                        name_starts_with_underscore = name.starts_with('_');
                    }
                }

                if is_top_level && !name_starts_with_underscore {
                    exported_count += 1;
                }
            }
        }

        if exported_count >= MONOLITHIC_EXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "monolithic_export_surface".to_string(),
                message: format!(
                    "Module exports {} symbols (threshold: {})",
                    exported_count, MONOLITHIC_EXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 3: Anemic module
        if total_definitions == ANEMIC_DEFINITION_THRESHOLD
            && !is_entry_file(file_path, ANEMIC_ENTRY_FILES)
        {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| PYTHON_DEFINITION_KINDS.contains(&c.kind()))
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
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::new();
        for i in 0..31 {
            src.push_str(&format!("def func_{}():\n    pass\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "def foo():\n    pass\ndef bar():\n    pass\nclass Baz:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("def func_{}():\n    pass\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn no_monolithic_for_private_symbols() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("def _private_func_{}():\n    pass\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn detects_anemic_module() {
        let src = "def only_function():\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
        let src = "def setup():\n    pass\n";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "__init__.py");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "def foo():\n    pass\ndef bar():\n    pass\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }
}
