use std::collections::HashMap;

use anyhow::Result;
use tree_sitter::Tree;

use crate::graph::CodeGraph;
use crate::language::Language;

use super::models::AuditFinding;
use super::pipelines;

/// Context passed to pipelines during graph-aware checking.
pub struct PipelineContext<'a> {
    pub tree: &'a Tree,
    pub source: &'a [u8],
    pub file_path: &'a str,
    pub id_counts: &'a HashMap<String, usize>,
    pub graph: Option<&'a CodeGraph>,
}

pub trait Pipeline: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding>;

    /// Check with pre-computed identifier occurrence counts.
    /// Pipelines that need identifier counts (e.g., dead_code) can override this
    /// to avoid recomputing them per-pipeline.
    fn check_with_ids(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        _id_counts: &HashMap<String, usize>,
    ) -> Vec<AuditFinding> {
        self.check(tree, source, file_path)
    }

    /// Check with full pipeline context including CodeGraph access.
    /// Default delegates to check_with_ids — pipelines that want graph access
    /// override this method. Zero breakage across existing pipelines.
    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        self.check_with_ids(ctx.tree, ctx.source, ctx.file_path, ctx.id_counts)
    }
}

pub fn pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::tech_debt_pipelines(),
        Language::Go => pipelines::go::tech_debt_pipelines(),
        Language::Python => pipelines::python::tech_debt_pipelines(),
        Language::Php => pipelines::php::tech_debt_pipelines(),
        Language::Java => pipelines::java::tech_debt_pipelines(),
        Language::JavaScript => pipelines::javascript::tech_debt_pipelines(),
        Language::TypeScript | Language::Tsx => {
            pipelines::typescript::tech_debt_pipelines(language)
        }
        Language::C => pipelines::c::tech_debt_pipelines(),
        Language::Cpp => pipelines::cpp::tech_debt_pipelines(),
        Language::CSharp => pipelines::csharp::tech_debt_pipelines(),
        _ => Ok(vec![]),
    }
}

pub fn complexity_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::complexity_pipelines(),
        Language::Go => pipelines::go::complexity_pipelines(),
        Language::Python => pipelines::python::complexity_pipelines(),
        Language::Php => pipelines::php::complexity_pipelines(),
        Language::Java => pipelines::java::complexity_pipelines(),
        Language::JavaScript => pipelines::javascript::complexity_pipelines(),
        Language::TypeScript | Language::Tsx => {
            pipelines::typescript::complexity_pipelines(language)
        }
        Language::C => pipelines::c::complexity_pipelines(),
        Language::Cpp => pipelines::cpp::complexity_pipelines(),
        Language::CSharp => pipelines::csharp::complexity_pipelines(),
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

pub fn code_style_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::code_style_pipelines(),
        Language::Go => pipelines::go::code_style_pipelines(),
        Language::Python => pipelines::python::code_style_pipelines(),
        Language::Php => pipelines::php::code_style_pipelines(),
        Language::Java => pipelines::java::code_style_pipelines(),
        Language::JavaScript => pipelines::javascript::code_style_pipelines(),
        Language::TypeScript | Language::Tsx => {
            pipelines::typescript::code_style_pipelines(language)
        }
        Language::C => pipelines::c::code_style_pipelines(),
        Language::Cpp => pipelines::cpp::code_style_pipelines(),
        Language::CSharp => pipelines::csharp::code_style_pipelines(),
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

pub fn security_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::security_pipelines(),
        Language::Go => pipelines::go::security_pipelines(),
        Language::Python => pipelines::python::security_pipelines(),
        Language::Php => pipelines::php::security_pipelines(),
        Language::Java => pipelines::java::security_pipelines(),
        Language::C => pipelines::c::security_pipelines(),
        Language::CSharp => pipelines::csharp::security_pipelines(),
        Language::JavaScript => pipelines::javascript::security_pipelines(Language::JavaScript),
        Language::TypeScript | Language::Tsx => pipelines::typescript::security_pipelines(language),
        Language::Cpp => pipelines::cpp::security_pipelines(),
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

pub fn scalability_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::scalability_pipelines(),
        Language::Go => pipelines::go::scalability_pipelines(),
        Language::Python => pipelines::python::scalability_pipelines(),
        Language::Php => pipelines::php::scalability_pipelines(),
        Language::Java => pipelines::java::scalability_pipelines(),
        Language::JavaScript => pipelines::javascript::scalability_pipelines(),
        Language::TypeScript | Language::Tsx => {
            pipelines::typescript::scalability_pipelines(language)
        }
        Language::C => pipelines::c::scalability_pipelines(),
        Language::Cpp => pipelines::cpp::scalability_pipelines(),
        Language::CSharp => pipelines::csharp::scalability_pipelines(),
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

pub fn architecture_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => pipelines::rust::architecture_pipelines(),
        Language::Go => pipelines::go::architecture_pipelines(),
        Language::Python => pipelines::python::architecture_pipelines(),
        Language::Php => pipelines::php::architecture_pipelines(),
        Language::Java => pipelines::java::architecture_pipelines(),
        Language::JavaScript => pipelines::javascript::architecture_pipelines(),
        Language::TypeScript | Language::Tsx => {
            pipelines::typescript::architecture_pipelines(language)
        }
        Language::C => pipelines::c::architecture_pipelines(),
        Language::Cpp => pipelines::cpp::architecture_pipelines(),
        Language::CSharp => pipelines::csharp::architecture_pipelines(),
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
