use std::collections::HashMap;

use anyhow::Result;
use tree_sitter::Tree;

use crate::graph::CodeGraph;
use crate::language::Language;

use super::models::AuditFinding;
use super::pipelines;

/// Context passed to legacy pipelines during graph-aware checking.
pub struct PipelineContext<'a> {
    pub tree: &'a Tree,
    pub source: &'a [u8],
    pub file_path: &'a str,
    pub id_counts: &'a HashMap<String, usize>,
    pub graph: Option<&'a CodeGraph>,
}

/// Context passed to graph-primary pipelines. Graph is required (not Option).
pub struct GraphPipelineContext<'a> {
    pub tree: &'a Tree,
    pub source: &'a [u8],
    pub file_path: &'a str,
    pub id_counts: &'a HashMap<String, usize>,
    pub graph: &'a CodeGraph,
}

/// Legacy pipeline trait — used by non-Python languages during migration.
pub trait Pipeline: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding>;

    fn check_with_ids(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        _id_counts: &HashMap<String, usize>,
    ) -> Vec<AuditFinding> {
        self.check(tree, source, file_path)
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        self.check_with_ids(ctx.tree, ctx.source, ctx.file_path, ctx.id_counts)
    }
}

/// Per-node pipeline — inherently per-node metrics (complexity, line counts).
/// No graph needed, always runs.
pub trait NodePipeline: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding>;
}

/// Graph-primary pipeline — requires CodeGraph for analysis.
/// Uses tree-sitter for AST access but graph is the primary analysis engine.
pub trait GraphPipeline: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding>;
}

/// Unified pipeline wrapper for engine dispatch.
pub enum AnyPipeline {
    Node(Box<dyn NodePipeline>),
    Graph(Box<dyn GraphPipeline>),
    Legacy(Box<dyn Pipeline>),
}

impl AnyPipeline {
    pub fn name(&self) -> &str {
        match self {
            AnyPipeline::Node(p) => p.name(),
            AnyPipeline::Graph(p) => p.name(),
            AnyPipeline::Legacy(p) => p.name(),
        }
    }
}

/// Wrap legacy `Box<dyn Pipeline>` results as `AnyPipeline::Legacy`.
fn wrap_legacy(pipelines: Result<Vec<Box<dyn Pipeline>>>) -> Result<Vec<AnyPipeline>> {
    Ok(pipelines?
        .into_iter()
        .map(|p| AnyPipeline::Legacy(p))
        .collect())
}

pub fn pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::tech_debt_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::tech_debt_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::tech_debt_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::tech_debt_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::tech_debt_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::tech_debt_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::tech_debt_pipelines(language))
        }
        Language::C => wrap_legacy(pipelines::c::tech_debt_pipelines()),
        Language::Cpp => wrap_legacy(pipelines::cpp::tech_debt_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::tech_debt_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn complexity_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::complexity_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::complexity_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::complexity_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::complexity_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::complexity_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::complexity_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::complexity_pipelines(language))
        }
        Language::C => wrap_legacy(pipelines::c::complexity_pipelines()),
        Language::Cpp => wrap_legacy(pipelines::cpp::complexity_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::complexity_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn supported_audit_languages() -> Vec<Language> {
    vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::C,
        Language::Cpp,
        Language::CSharp,
    ]
}

pub fn supported_complexity_languages() -> Vec<Language> {
    vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::C,
        Language::Cpp,
        Language::CSharp,
    ]
}

pub fn code_style_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::code_style_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::code_style_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::code_style_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::code_style_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::code_style_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::code_style_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::code_style_pipelines(language))
        }
        Language::C => wrap_legacy(pipelines::c::code_style_pipelines()),
        Language::Cpp => wrap_legacy(pipelines::cpp::code_style_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::code_style_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn supported_code_style_languages() -> Vec<Language> {
    vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::C,
        Language::Cpp,
        Language::CSharp,
    ]
}

pub fn security_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::security_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::security_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::security_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::security_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::security_pipelines()),
        Language::C => wrap_legacy(pipelines::c::security_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::security_pipelines()),
        Language::JavaScript => {
            wrap_legacy(pipelines::javascript::security_pipelines(Language::JavaScript))
        }
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::security_pipelines(language))
        }
        Language::Cpp => wrap_legacy(pipelines::cpp::security_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn supported_security_languages() -> Vec<Language> {
    vec![
        Language::C,
        Language::Cpp,
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::CSharp,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
    ]
}

pub fn scalability_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::scalability_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::scalability_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::scalability_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::scalability_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::scalability_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::scalability_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::scalability_pipelines(language))
        }
        Language::C => wrap_legacy(pipelines::c::scalability_pipelines()),
        Language::Cpp => wrap_legacy(pipelines::cpp::scalability_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::scalability_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn supported_scalability_languages() -> Vec<Language> {
    vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::C,
        Language::Cpp,
        Language::CSharp,
    ]
}

pub fn architecture_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::architecture_pipelines(),
        Language::Rust => wrap_legacy(pipelines::rust::architecture_pipelines()),
        Language::Go => wrap_legacy(pipelines::go::architecture_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::architecture_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::architecture_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::architecture_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::typescript::architecture_pipelines(language))
        }
        Language::C => wrap_legacy(pipelines::c::architecture_pipelines()),
        Language::Cpp => wrap_legacy(pipelines::cpp::architecture_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::architecture_pipelines()),
        _ => Ok(vec![]),
    }
}

pub fn supported_architecture_languages() -> Vec<Language> {
    vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::Php,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::C,
        Language::Cpp,
        Language::CSharp,
    ]
}
