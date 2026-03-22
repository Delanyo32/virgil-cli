use std::collections::HashSet;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::audit::project_index::ProjectIndex;

/// Entry-point file names that should not be flagged as dead exports.
const ENTRY_POINT_NAMES: &[&str] = &["main", "lib", "mod", "index", "__init__", "__main__"];

pub struct DeadExportsAnalyzer;

impl ProjectAnalyzer for DeadExportsAnalyzer {
    fn name(&self) -> &str {
        "dead_exports"
    }

    fn description(&self) -> &str {
        "Detect exported symbols that are never imported by any other file"
    }

    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding> {
        // Build set of all internally-imported names across all files
        let mut imported_names: HashSet<String> = HashSet::new();
        for entry in index.files.values() {
            for import in &entry.imports {
                if !import.is_external {
                    imported_names.insert(import.imported_name.clone());
                    // Also add the local name in case of aliasing
                    imported_names.insert(import.local_name.clone());
                }
            }
        }

        // Wildcard imports mean we can't know what's used
        let has_wildcard = imported_names.contains("*");

        let mut findings = Vec::new();

        for entry in index.files.values() {
            // Skip entry-point files
            if is_entry_point(&entry.path) {
                continue;
            }

            for symbol in &entry.exported_symbols {
                // Skip main functions
                if symbol.name == "main" || symbol.name == "__init__" {
                    continue;
                }

                // If there are wildcard imports, we can't prove a symbol is dead
                if has_wildcard {
                    continue;
                }

                if !imported_names.contains(&symbol.name) {
                    findings.push(AuditFinding {
                        file_path: entry.path.clone(),
                        line: symbol.start_line,
                        column: 1,
                        severity: "info".to_string(),
                        pipeline: "dead_exports".to_string(),
                        pattern: "dead_export".to_string(),
                        message: format!(
                            "Exported {} '{}' is not imported by any other file in the project",
                            symbol.kind, symbol.name
                        ),
                        snippet: symbol.signature.clone().unwrap_or_default(),
                    });
                }
            }
        }

        findings
    }
}

fn is_entry_point(path: &str) -> bool {
    let file_stem = path
        .rsplit_once('/')
        .map(|(_, f)| f)
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(path);

    ENTRY_POINT_NAMES.contains(&file_stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::project_index::{ExportedSymbol, FileEntry};
    use crate::language::Language;
    use crate::models::{ImportInfo, SymbolKind};

    #[test]
    fn detects_dead_export() {
        let mut index = ProjectIndex::new();
        index.files.insert(
            "src/utils.rs".into(),
            FileEntry {
                path: "src/utils.rs".into(),
                language: Language::Rust,
                line_count: 10,
                symbol_count: 1,
                exported_symbols: vec![ExportedSymbol {
                    name: "format_date".into(),
                    kind: SymbolKind::Function,
                    signature: Some("pub fn format_date()".into()),
                    start_line: 1,
                }],
                imports: vec![],
            },
        );
        index.files.insert(
            "src/main.rs".into(),
            FileEntry {
                path: "src/main.rs".into(),
                language: Language::Rust,
                line_count: 5,
                symbol_count: 1,
                exported_symbols: vec![],
                imports: vec![], // Does NOT import format_date
            },
        );

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "dead_export");
        assert!(findings[0].message.contains("format_date"));
    }

    #[test]
    fn no_finding_when_imported() {
        let mut index = ProjectIndex::new();
        index.files.insert(
            "src/utils.rs".into(),
            FileEntry {
                path: "src/utils.rs".into(),
                language: Language::Rust,
                line_count: 10,
                symbol_count: 1,
                exported_symbols: vec![ExportedSymbol {
                    name: "format_date".into(),
                    kind: SymbolKind::Function,
                    signature: None,
                    start_line: 1,
                }],
                imports: vec![],
            },
        );
        index.files.insert(
            "src/handler.rs".into(),
            FileEntry {
                path: "src/handler.rs".into(),
                language: Language::Rust,
                line_count: 5,
                symbol_count: 1,
                exported_symbols: vec![],
                imports: vec![ImportInfo {
                    source_file: "src/handler.rs".into(),
                    module_specifier: "crate::utils::format_date".into(),
                    imported_name: "format_date".into(),
                    local_name: "format_date".into(),
                    kind: "use".into(),
                    is_type_only: false,
                    line: 1,
                    is_external: false,
                }],
            },
        );

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_entry_points() {
        let mut index = ProjectIndex::new();
        index.files.insert(
            "src/main.rs".into(),
            FileEntry {
                path: "src/main.rs".into(),
                language: Language::Rust,
                line_count: 10,
                symbol_count: 1,
                exported_symbols: vec![ExportedSymbol {
                    name: "run".into(),
                    kind: SymbolKind::Function,
                    signature: None,
                    start_line: 1,
                }],
                imports: vec![],
            },
        );

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }
}
