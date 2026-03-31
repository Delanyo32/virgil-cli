pub mod c_lang;
pub mod cpp;
pub mod csharp;
pub mod go;
pub mod java;
pub mod php;
pub mod python;
pub mod rust_lang;
pub mod typescript;

use anyhow::Result;
use tree_sitter::Node;

use crate::language::Language;

use super::cfg::FunctionCfg;

/// Trait for language-specific CFG builders.
pub trait CfgBuilder: Send + Sync {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg>;
}

/// Get the appropriate CFG builder for a language.
pub fn cfg_builder_for_language(language: Language) -> Option<Box<dyn CfgBuilder>> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            Some(Box::new(typescript::TypeScriptCfgBuilder))
        }
        Language::Python => Some(Box::new(python::PythonCfgBuilder)),
        Language::Rust => Some(Box::new(rust_lang::RustCfgBuilder)),
        Language::Go => Some(Box::new(go::GoCfgBuilder)),
        Language::Java => Some(Box::new(java::JavaCfgBuilder)),
        Language::C => Some(Box::new(c_lang::CCfgBuilder)),
        Language::Cpp => Some(Box::new(cpp::CppCfgBuilder)),
        Language::CSharp => Some(Box::new(csharp::CSharpCfgBuilder)),
        Language::Php => Some(Box::new(php::PhpCfgBuilder)),
    }
}
