use std::collections::HashMap;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::audit::project_index::ProjectIndex;
use crate::models::SymbolKind;

pub struct DuplicateSymbolsAnalyzer;

impl ProjectAnalyzer for DuplicateSymbolsAnalyzer {
    fn name(&self) -> &str {
        "cross_file_duplicates"
    }

    fn description(&self) -> &str {
        "Detect exported symbols with identical name, kind, and signature across files"
    }

    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding> {
        // Group exported symbols by (name, kind, signature) triple
        let mut groups: HashMap<(String, SymbolKind, String), Vec<(String, u32)>> = HashMap::new();

        for entry in index.files.values() {
            for symbol in &entry.exported_symbols {
                let sig = symbol.signature.clone().unwrap_or_default();
                let key = (symbol.name.clone(), symbol.kind, sig);
                groups
                    .entry(key)
                    .or_default()
                    .push((entry.path.clone(), symbol.start_line));
            }
        }

        let mut findings = Vec::new();

        for ((name, kind, _sig), locations) in &groups {
            if locations.len() < 2 {
                continue;
            }

            let other_files: Vec<String> = locations.iter().map(|(p, _)| p.clone()).collect();
            let message = format!(
                "Cross-file duplicate: {} '{}' has identical signature in {} files: {}",
                kind,
                name,
                other_files.len(),
                other_files.join(", ")
            );

            for (file_path, line) in locations {
                findings.push(AuditFinding {
                    file_path: file_path.clone(),
                    line: *line,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "cross_file_duplicates".to_string(),
                    pattern: "cross_file_duplicate".to_string(),
                    message: message.clone(),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::project_index::{ExportedSymbol, FileEntry};
    use crate::language::Language;

    #[test]
    fn detects_cross_file_duplicates() {
        let mut index = ProjectIndex::new();
        let sym = |path: &str| FileEntry {
            path: path.into(),
            language: Language::Rust,
            line_count: 10,
            symbol_count: 1,
            exported_symbols: vec![ExportedSymbol {
                name: "parse_config".into(),
                kind: SymbolKind::Function,
                signature: Some("pub fn parse_config(path: &str) -> Config".into()),
                start_line: 1,
            }],
            imports: vec![],
        };

        index.files.insert("src/a.rs".into(), sym("src/a.rs"));
        index.files.insert("src/b.rs".into(), sym("src/b.rs"));

        let analyzer = DuplicateSymbolsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert_eq!(findings.len(), 2); // One per file
        assert!(findings
            .iter()
            .all(|f| f.pattern == "cross_file_duplicate"));
    }

    #[test]
    fn no_duplicate_with_different_signatures() {
        let mut index = ProjectIndex::new();
        index.files.insert(
            "src/a.rs".into(),
            FileEntry {
                path: "src/a.rs".into(),
                language: Language::Rust,
                line_count: 10,
                symbol_count: 1,
                exported_symbols: vec![ExportedSymbol {
                    name: "parse".into(),
                    kind: SymbolKind::Function,
                    signature: Some("pub fn parse(s: &str)".into()),
                    start_line: 1,
                }],
                imports: vec![],
            },
        );
        index.files.insert(
            "src/b.rs".into(),
            FileEntry {
                path: "src/b.rs".into(),
                language: Language::Rust,
                line_count: 10,
                symbol_count: 1,
                exported_symbols: vec![ExportedSymbol {
                    name: "parse".into(),
                    kind: SymbolKind::Function,
                    signature: Some("pub fn parse(n: i32)".into()),
                    start_line: 1,
                }],
                imports: vec![],
            },
        );

        let analyzer = DuplicateSymbolsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }
}
