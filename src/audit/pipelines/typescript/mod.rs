pub mod primitives;

pub mod any_escape_hatch;
pub mod enum_usage;
pub mod implicit_any;
pub mod leaking_impl_types;
pub mod mutable_types;
pub mod optional_everything;
pub mod record_string_any;
pub mod type_assertions;
pub mod type_duplication;
pub mod unchecked_index_access;
pub mod unconstrained_generics;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

pub fn tech_debt_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(any_escape_hatch::AnyEscapeHatchPipeline::new(language)?),
        Box::new(type_assertions::TypeAssertionsPipeline::new(language)?),
        Box::new(optional_everything::OptionalEverythingPipeline::new(language)?),
        Box::new(type_duplication::TypeDuplicationPipeline::new(language)?),
        Box::new(record_string_any::RecordStringAnyPipeline::new(language)?),
        Box::new(enum_usage::EnumUsagePipeline::new(language)?),
        Box::new(implicit_any::ImplicitAnyPipeline::new(language)?),
        Box::new(unchecked_index_access::UncheckedIndexAccessPipeline::new(language)?),
        Box::new(mutable_types::MutableTypesPipeline::new(language)?),
        Box::new(unconstrained_generics::UnconstrainedGenericsPipeline::new(language)?),
        Box::new(leaking_impl_types::LeakingImplTypesPipeline::new(language)?),
    ])
}

pub fn complexity_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new(language)?),
        Box::new(function_length::FunctionLengthPipeline::new(language)?),
        Box::new(cognitive::CognitiveComplexityPipeline::new(language)?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new(language)?),
    ])
}
