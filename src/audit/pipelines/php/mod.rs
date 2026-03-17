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

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(deprecated_mysql_api::DeprecatedMysqlApiPipeline::new()?),
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(error_suppression::ErrorSuppressionPipeline::new()?),
        Box::new(missing_type_declarations::MissingTypeDeclarationsPipeline::new()?),
        Box::new(god_class::GodClassPipeline::new()?),
        Box::new(extract_usage::ExtractUsagePipeline::new()?),
        Box::new(silent_exception::SilentExceptionPipeline::new()?),
        Box::new(unsafe_include::UnsafeIncludePipeline::new()?),
        Box::new(unescaped_output::UnescapedOutputPipeline::new()?),
        Box::new(logic_in_views::LogicInViewsPipeline::new()?),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new()?),
        Box::new(function_length::FunctionLengthPipeline::new()?),
        Box::new(cognitive::CognitiveComplexityPipeline::new()?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new()?),
    ])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new()?),
        Box::new(duplicate_code::DuplicateCodePipeline::new()?),
        Box::new(coupling::CouplingPipeline::new()?),
    ])
}
