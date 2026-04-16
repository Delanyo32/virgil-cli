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

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;


use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines;
use crate::language::Language;
use anyhow::Result;

pub fn tech_debt_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(any_escape_hatch::AnyEscapeHatchPipeline::new(language)?),
        Box::new(type_assertions::TypeAssertionsPipeline::new(language)?),
        Box::new(optional_everything::OptionalEverythingPipeline::new(
            language,
        )?),
        Box::new(type_duplication::TypeDuplicationPipeline::new(language)?),
        Box::new(record_string_any::RecordStringAnyPipeline::new(language)?),
        Box::new(enum_usage::EnumUsagePipeline::new(language)?),
        Box::new(implicit_any::ImplicitAnyPipeline::new(language)?),
        Box::new(unchecked_index_access::UncheckedIndexAccessPipeline::new(
            language,
        )?),
        Box::new(mutable_types::MutableTypesPipeline::new(language)?),
        Box::new(unconstrained_generics::UnconstrainedGenericsPipeline::new(
            language,
        )?),
        Box::new(leaking_impl_types::LeakingImplTypesPipeline::new(language)?),
    ])
}

pub fn complexity_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn code_style_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new(language)?),
        Box::new(duplicate_code::DuplicateCodePipeline::new(language)?),
        Box::new(coupling::CouplingPipeline::new(language)?),
    ])
}

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    pipelines::javascript::security_pipelines(language)
}

pub fn scalability_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

