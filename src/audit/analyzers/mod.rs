use super::project_analyzer::ProjectAnalyzer;

/// Project analyzers for the Architecture category.
/// All architecture analysis is now handled by JSON pipelines.
pub fn architecture_analyzers() -> Vec<Box<dyn ProjectAnalyzer>> {
    vec![]
}

/// Project analyzers for the CodeStyle category.
/// All code-style analysis is now handled by JSON pipelines.
pub fn code_style_analyzers() -> Vec<Box<dyn ProjectAnalyzer>> {
    vec![]
}
