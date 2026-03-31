use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};

/// Entry-point file names that should not be flagged as dead exports.
const ENTRY_POINT_NAMES: &[&str] = &["main", "lib", "mod", "index", "__init__", "__main__"];

pub struct DeadExportsAnalyzer;

impl ProjectAnalyzer for DeadExportsAnalyzer {
    fn name(&self) -> &str {
        "dead_exports"
    }

    fn description(&self) -> &str {
        "Detect exported symbols that are never referenced by any other file"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        for sym_idx in graph.graph.node_indices() {
            let (name, kind, file_path, start_line, exported) = match &graph.graph[sym_idx] {
                NodeWeight::Symbol {
                    name,
                    kind,
                    file_path,
                    start_line,
                    exported,
                    ..
                } => (name, kind, file_path, *start_line, *exported),
                _ => continue,
            };

            if !exported {
                continue;
            }

            // Skip entry-point files
            if is_entry_point(file_path) {
                continue;
            }

            // Skip framework-dispatched files (web handlers, tasks, tests, migrations)
            if is_framework_dispatched_file(file_path) {
                continue;
            }

            // Skip main functions
            if name == "main" || name == "__init__" {
                continue;
            }

            // Check if any incoming Calls edge comes from a symbol in a different file
            let has_cross_file_caller = graph
                .graph
                .edges_directed(sym_idx, Direction::Incoming)
                .any(|edge| {
                    if !matches!(edge.weight(), EdgeWeight::Calls) {
                        return false;
                    }
                    // Check if caller is in a different file
                    match &graph.graph[edge.source()] {
                        NodeWeight::Symbol {
                            file_path: caller_file,
                            ..
                        } => caller_file != file_path,
                        _ => false,
                    }
                });

            if !has_cross_file_caller {
                findings.push(AuditFinding {
                    file_path: file_path.clone(),
                    line: start_line,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "dead_exports".to_string(),
                    pattern: "dead_export".to_string(),
                    message: format!(
                        "Exported {} '{}' is not referenced by any other file in the project",
                        kind, name
                    ),
                    snippet: String::new(),
                });
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

/// File stems for framework-dispatched modules (web handlers, admin, signals, etc.)
const FRAMEWORK_FILE_STEMS: &[&str] = &[
    "api",
    "views",
    "routes",
    "tasks",
    "handlers",
    "endpoints",
    "commands",
    "admin",
    "signals",
    "middleware",
    "urls",
    "wsgi",
    "asgi",
    "conftest",
    "manage",
];

/// Directory segments that indicate framework-managed or non-library code.
const FRAMEWORK_DIR_SEGMENTS: &[&str] = &[
    "tests",
    "test",
    "migrations",
    "management",
    "commands",
];

/// Returns `true` if the file lives in a framework-dispatched context where
/// exported symbols are typically invoked by the framework (or test runner)
/// rather than imported directly by other project code.
fn is_framework_dispatched_file(path: &str) -> bool {
    // Check file stem against known framework file names
    let file_name = path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path);
    let file_stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);

    if FRAMEWORK_FILE_STEMS.contains(&file_stem) {
        return true;
    }

    // Check for test file patterns: test_*.py, *_test.py
    if file_stem.starts_with("test_") || file_stem.ends_with("_test") {
        return true;
    }

    // Check if any directory segment matches a framework directory
    for segment in path.split('/') {
        if FRAMEWORK_DIR_SEGMENTS.contains(&segment) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;
    use crate::models::SymbolKind;

    #[test]
    fn detects_dead_export() {
        let mut graph = CodeGraph::new();

        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/utils.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/utils.rs".to_string(), file_idx);

        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "format_date".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/utils.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });
        graph
            .graph
            .add_edge(sym_idx, file_idx, EdgeWeight::DefinedIn);

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "dead_export");
        assert!(findings[0].message.contains("format_date"));
    }

    #[test]
    fn no_finding_when_called_cross_file() {
        let mut graph = CodeGraph::new();

        let file_a = graph.graph.add_node(NodeWeight::File {
            path: "src/utils.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/utils.rs".to_string(), file_a);

        let sym_a = graph.graph.add_node(NodeWeight::Symbol {
            name: "format_date".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/utils.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });

        let file_b = graph.graph.add_node(NodeWeight::File {
            path: "src/handler.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/handler.rs".to_string(), file_b);

        let sym_b = graph.graph.add_node(NodeWeight::Symbol {
            name: "handle".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/handler.rs".to_string(),
            start_line: 1,
            end_line: 10,
            exported: false,
        });

        // sym_b calls sym_a
        graph.graph.add_edge(sym_b, sym_a, EdgeWeight::Calls);

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_entry_points() {
        let mut graph = CodeGraph::new();

        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/main.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/main.rs".to_string(), file_idx);

        let _sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "run".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_framework_dispatched_files() {
        let mut graph = CodeGraph::new();

        // views.py — Django view module
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "app/views.py".to_string(),
            language: Language::Python,
        });
        graph
            .file_nodes
            .insert("app/views.py".to_string(), file_idx);

        let _sym = graph.graph.add_node(NodeWeight::Symbol {
            name: "index".to_string(),
            kind: SymbolKind::Function,
            file_path: "app/views.py".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty(), "views.py should be skipped");
    }

    #[test]
    fn is_framework_dispatched_file_matches_stems() {
        // Framework file stems
        assert!(is_framework_dispatched_file("app/views.py"));
        assert!(is_framework_dispatched_file("app/routes.py"));
        assert!(is_framework_dispatched_file("app/tasks.py"));
        assert!(is_framework_dispatched_file("app/handlers.py"));
        assert!(is_framework_dispatched_file("app/endpoints.py"));
        assert!(is_framework_dispatched_file("app/commands.py"));
        assert!(is_framework_dispatched_file("app/admin.py"));
        assert!(is_framework_dispatched_file("app/signals.py"));
        assert!(is_framework_dispatched_file("app/middleware.py"));
        assert!(is_framework_dispatched_file("app/urls.py"));
        assert!(is_framework_dispatched_file("app/wsgi.py"));
        assert!(is_framework_dispatched_file("app/asgi.py"));
        assert!(is_framework_dispatched_file("conftest.py"));
        assert!(is_framework_dispatched_file("manage.py"));
        assert!(is_framework_dispatched_file("project/api.py"));
    }

    #[test]
    fn is_framework_dispatched_file_matches_test_patterns() {
        assert!(is_framework_dispatched_file("app/test_views.py"));
        assert!(is_framework_dispatched_file("app/models_test.py"));
        // But not a file that merely contains "test" in the middle
        assert!(!is_framework_dispatched_file("app/contest.py"));
    }

    #[test]
    fn is_framework_dispatched_file_matches_dir_segments() {
        assert!(is_framework_dispatched_file("project/tests/test_foo.py"));
        assert!(is_framework_dispatched_file("project/test/helpers.py"));
        assert!(is_framework_dispatched_file("app/migrations/0001_initial.py"));
        assert!(is_framework_dispatched_file("app/management/commands/seed.py"));
        assert!(is_framework_dispatched_file("app/commands/deploy.py"));
    }

    #[test]
    fn is_framework_dispatched_file_rejects_normal_files() {
        assert!(!is_framework_dispatched_file("app/models.py"));
        assert!(!is_framework_dispatched_file("src/utils.rs"));
        assert!(!is_framework_dispatched_file("lib/helpers.ts"));
    }
}
