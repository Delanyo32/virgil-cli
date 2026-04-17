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
        .map(AnyPipeline::Legacy)
        .collect())
}

pub fn pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::tech_debt_pipelines(),
        Language::Go => pipelines::go::tech_debt_pipelines(),
        Language::Php => pipelines::php::tech_debt_pipelines(),
        Language::Java => pipelines::java::tech_debt_pipelines(),
        Language::JavaScript => pipelines::javascript::tech_debt_pipelines(),
        Language::TypeScript | Language::Tsx => pipelines::javascript::tech_debt_pipelines(),
        Language::CSharp => pipelines::csharp::tech_debt_pipelines(),
        _ => Ok(vec![]),
    }
}

pub fn complexity_pipelines_for_language(language: Language) -> Result<Vec<AnyPipeline>> {
    match language {
        Language::Python => pipelines::python::complexity_pipelines(),
        Language::Go => wrap_legacy(pipelines::go::complexity_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::complexity_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::complexity_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::complexity_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::javascript::complexity_pipelines())
        }
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
        Language::Go => wrap_legacy(pipelines::go::code_style_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::code_style_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::code_style_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::code_style_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::javascript::code_style_pipelines())
        }
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
        Language::Go => wrap_legacy(pipelines::go::security_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::security_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::security_pipelines()),
        Language::CSharp => wrap_legacy(pipelines::csharp::security_pipelines()),
        Language::JavaScript => {
            wrap_legacy(pipelines::javascript::security_pipelines(Language::JavaScript))
        }
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::javascript::security_pipelines(language))
        }
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
        Language::Go => wrap_legacy(pipelines::go::scalability_pipelines()),
        Language::Php => wrap_legacy(pipelines::php::scalability_pipelines()),
        Language::Java => wrap_legacy(pipelines::java::scalability_pipelines()),
        Language::JavaScript => wrap_legacy(pipelines::javascript::scalability_pipelines()),
        Language::TypeScript | Language::Tsx => {
            wrap_legacy(pipelines::javascript::scalability_pipelines())
        }
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

