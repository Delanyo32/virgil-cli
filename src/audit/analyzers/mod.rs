pub mod coupling;
pub mod dead_exports;
pub mod duplicate_symbols;

use super::project_analyzer::ProjectAnalyzer;

/// Project analyzers for the Architecture category.
pub fn architecture_analyzers() -> Vec<Box<dyn ProjectAnalyzer>> {
    vec![
        Box::new(coupling::CouplingAnalyzer),
    ]
}

/// Project analyzers for the CodeStyle category.
pub fn code_style_analyzers() -> Vec<Box<dyn ProjectAnalyzer>> {
    vec![
        Box::new(dead_exports::DeadExportsAnalyzer),
        Box::new(duplicate_symbols::DuplicateSymbolsAnalyzer),
    ]
}
