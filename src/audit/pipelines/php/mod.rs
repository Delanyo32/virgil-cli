pub mod primitives;

pub mod deprecated_mysql_api;
pub mod error_suppression;
pub mod extract_usage;
pub mod god_class;
pub mod logic_in_views;
pub mod missing_type_declarations;
pub mod silent_exception;
pub mod sql_injection;
pub mod unescaped_output;
pub mod unsafe_include;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod command_injection;
pub mod insecure_deserialization;
pub mod session_auth;
pub mod ssrf;
pub mod type_juggling;

pub mod memory_leak_indicators;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Node(Box::new(deprecated_mysql_api::DeprecatedMysqlApiPipeline::new()?)),
        AnyPipeline::Node(Box::new(error_suppression::ErrorSuppressionPipeline::new()?)),
        AnyPipeline::Node(Box::new(missing_type_declarations::MissingTypeDeclarationsPipeline::new()?)),
        AnyPipeline::Node(Box::new(god_class::GodClassPipeline::new()?)),
        AnyPipeline::Node(Box::new(extract_usage::ExtractUsagePipeline::new()?)),
        AnyPipeline::Node(Box::new(silent_exception::SilentExceptionPipeline::new()?)),
        AnyPipeline::Node(Box::new(logic_in_views::LogicInViewsPipeline::new()?)),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new()?),
        Box::new(duplicate_code::DuplicateCodePipeline::new()?),
        Box::new(coupling::CouplingPipeline::new()?),
    ])
}

pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(unsafe_include::UnsafeIncludePipeline::new()?),
        Box::new(unescaped_output::UnescapedOutputPipeline::new()?),
        Box::new(command_injection::CommandInjectionPipeline::new()?),
        Box::new(insecure_deserialization::InsecureDeserializationPipeline::new()?),
        Box::new(type_juggling::TypeJugglingPipeline::new()?),
        Box::new(ssrf::SsrfPipeline::new()?),
        Box::new(session_auth::SessionAuthPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}

