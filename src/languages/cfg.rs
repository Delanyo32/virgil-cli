//! CFG builder trait — implemented per-language under `languages/<lang>/cfg.rs`.

use anyhow::Result;
use tree_sitter::Node;

use crate::graph::cfg::FunctionCfg;
use crate::language::Language;

/// Trait for language-specific CFG builders.
pub trait CfgBuilder: Send + Sync {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg>;
}

/// Get the appropriate CFG builder for a language.
pub fn cfg_builder_for_language(language: Language) -> Option<Box<dyn CfgBuilder>> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            Some(Box::new(super::typescript::TypeScriptCfgBuilder))
        }
        Language::Python => Some(Box::new(super::python::PythonCfgBuilder)),
        Language::Rust => Some(Box::new(super::rust_lang::RustCfgBuilder)),
        Language::Go => Some(Box::new(super::go::GoCfgBuilder)),
        Language::Java => Some(Box::new(super::java::JavaCfgBuilder)),
        Language::C => Some(Box::new(super::c_lang::CCfgBuilder)),
        Language::Cpp => Some(Box::new(super::cpp::CppCfgBuilder)),
        Language::CSharp => Some(Box::new(super::csharp::CSharpCfgBuilder)),
        Language::Php => Some(Box::new(super::php::PhpCfgBuilder)),
    }
}
